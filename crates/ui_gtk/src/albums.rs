// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
};

use gtk::prelude::*;
use gtk::{gdk, glib};
use sustain_app_runtime::{ApplicationCommand, PlaybackCommand, Track, TrackId};

use super::{
    PlaybackChangedCallback, SharedRuntime, artwork_color::ArtworkPalette,
    command_controller::SharedCommandController, track_context::TrackRowContextMenu,
};
use model::{AlbumViewModel, album_subtitle, group_albums};
use track_list::AlbumTrackListView;

mod model;
mod track_list;

#[derive(Clone)]
pub(crate) struct AlbumsView {
    scroller: gtk::ScrolledWindow,
    container: gtk::Box,
    runtime: SharedRuntime,
    command_controller: SharedCommandController,
    playback_changed: PlaybackChangedCallback,
    context_menu: TrackRowContextMenu,
    albums: Rc<RefCell<Vec<AlbumViewModel>>>,
    selected_album: Rc<Cell<Option<usize>>>,
    selected_tile: Rc<RefCell<Option<gtk::Button>>>,
    visible_columns: Rc<Cell<usize>>,
    last_width: Rc<Cell<i32>>,
    artwork_cache: Rc<RefCell<BTreeMap<PathBuf, CachedArtwork>>>,
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    live_track_lists: Rc<RefCell<Vec<AlbumTrackListView>>>,
}

#[derive(Clone, Default)]
struct CachedArtwork {
    texture: Option<gdk::Texture>,
    palette: Option<ArtworkPalette>,
}

const ALBUM_TILE_WIDTH: i32 = 150;
const ALBUM_TILE_HORIZONTAL_PADDING: i32 = 16;
const ALBUM_TILE_MIN_WIDTH: i32 = ALBUM_TILE_WIDTH + ALBUM_TILE_HORIZONTAL_PADDING;
const ALBUM_TILE_COVER_SIZE: i32 = 132;
const ALBUM_GRID_MARGIN: i32 = 14;
const ALBUM_GRID_ROW_SPACING: i32 = 12;
const ALBUM_GRID_COLUMN_SPACING: i32 = 16;
const ALBUM_DETAIL_ARTWORK_SIZE: i32 = ALBUM_TILE_COVER_SIZE * 3;
const ALBUM_DETAIL_ARROW_WIDTH: i32 = 36;
const ALBUM_DETAIL_ARROW_HEIGHT: i32 = 18;
// One-pixel bleed below the triangle's base. The arrow row is laid out as
// an overlay on top of the detail panel so this bleed extends one row of
// arrow-coloured pixels into the panel's opaque background. Any sub-pixel
// transparency at the arrow texture's bottom edge — common when the
// scroller lands on a fractional offset — should then composite onto the
// panel's same-coloured pixels instead of revealing the window
// background.
// NOTE: Claude failed to fully eliminate the seam — a faint line under
// the arrow still appears intermittently, especially during scrolling.
const ALBUM_DETAIL_ARROW_BLEED: i32 = 1;
const ALBUM_COVER_PLACEHOLDER_ICON: &str = "image-missing-symbolic";

impl AlbumsView {
    pub(crate) fn new(
        runtime: SharedRuntime,
        command_controller: SharedCommandController,
        playback_changed: PlaybackChangedCallback,
        context_menu: TrackRowContextMenu,
    ) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, ALBUM_GRID_ROW_SPACING);
        container.add_css_class("albums-grid");
        container.set_margin_top(ALBUM_GRID_MARGIN);
        container.set_margin_bottom(ALBUM_GRID_MARGIN);
        container.set_hexpand(true);
        container.set_vexpand(false);

        let scroller = gtk::ScrolledWindow::new();
        scroller.add_css_class("albums-view");
        scroller.set_vexpand(true);
        scroller.set_hexpand(true);
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_child(Some(&container));

        let view = Self {
            scroller,
            container,
            runtime,
            command_controller,
            playback_changed,
            context_menu,
            albums: Rc::new(RefCell::new(Vec::new())),
            selected_album: Rc::new(Cell::new(None)),
            selected_tile: Rc::new(RefCell::new(None)),
            visible_columns: Rc::new(Cell::new(1)),
            last_width: Rc::new(Cell::new(0)),
            artwork_cache: Rc::new(RefCell::new(BTreeMap::new())),
            playing_track_id: Rc::new(Cell::new(None)),
            live_track_lists: Rc::new(RefCell::new(Vec::new())),
        };
        view.replace_tracks(view.runtime.borrow().library_tracks().to_vec());
        view.install_width_watcher();
        view
    }

    pub(crate) fn widget(&self) -> gtk::ScrolledWindow {
        self.scroller.clone()
    }

    pub(crate) fn replace_tracks(&self, tracks: Vec<Track>) {
        self.albums.replace(group_albums(&tracks));
        self.selected_album.set(None);
        self.artwork_cache.borrow_mut().clear();
        self.visible_columns
            .set(columns_for_width(self.scroller.allocated_width()));
        self.rebuild();
    }

    pub(crate) fn set_playing_track_id(&self, playing_track_id: Option<TrackId>) {
        if self.playing_track_id.get() == playing_track_id {
            return;
        }
        self.playing_track_id.set(playing_track_id);
        for list in self.live_track_lists.borrow().iter() {
            list.set_playing_track_id(playing_track_id);
        }
    }

    /// Selects the album containing the given track, expands its detail panel,
    /// and brings the tile into view. Returns `false` when no album in the
    /// current grouping holds the track.
    pub(crate) fn reveal_album_for_track(&self, track_id: TrackId) -> bool {
        let album_index = {
            let albums = self.albums.borrow();
            albums
                .iter()
                .position(|album| album.tracks.iter().any(|track| track.id == track_id))
        };
        let Some(album_index) = album_index else {
            return false;
        };
        self.selected_album.set(Some(album_index));
        self.rebuild();
        self.scroll_selected_tile_to_top();
        if let Some(tile) = self.selected_tile.borrow().clone() {
            // Keep keyboard focus in the grid after a context-menu reveal so
            // arrow-key nav has a starting point. Scroll is handled above.
            glib::idle_add_local_once(move || {
                tile.grab_focus();
            });
        }
        true
    }

    /// Scrolls the grid so the selected album's tile row sits at the top of
    /// the viewport, leaving the full screen below for the expanded detail
    /// panel.
    ///
    /// Hooks the frame clock's `after-paint` signal so the tile's bounds are
    /// read after the current frame's UPDATE → LAYOUT → PAINT cycle has
    /// completed. An idle callback is not deterministic enough here: it can
    /// fire before the next frame's LAYOUT phase depending on whether a
    /// frame happens to be due, and in that pre-layout window
    /// `compute_bounds` on a freshly-added widget returns a zero-sized rect
    /// that would silently scroll the viewport to `value = 0`.
    // SUSPECTED FLAKINESS: occasional reports of the viewport still landing
    // at the top under rapid clicks; the assumption that `after-paint`
    // always fires with a finalized allocation has not been independently
    // verified, and a concurrent rebuild (e.g. width watcher) could swap
    // the tile out between registration and emission.
    fn scroll_selected_tile_to_top(&self) {
        let Some(tile) = self.selected_tile.borrow().clone() else {
            return;
        };
        let Some(frame_clock) = self.scroller.frame_clock() else {
            return;
        };
        let scroller = self.scroller.clone();
        let container = self.container.clone();

        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
        let handler_for_callback = handler.clone();
        let id = frame_clock.connect_after_paint(move |fc| {
            if let Some(id) = handler_for_callback.borrow_mut().take() {
                fc.disconnect(id);
            }
            let Some(bounds) = tile.compute_bounds(&container) else {
                return;
            };
            scroller
                .vadjustment()
                .set_value(f64::from(bounds.y()).max(0.0));
        });
        *handler.borrow_mut() = Some(id);
    }

    fn install_width_watcher(&self) {
        let view = self.clone();
        self.scroller.add_tick_callback(move |scroller, _clock| {
            let width = scroller.allocated_width();
            if width > 0 && view.last_width.replace(width) != width {
                let columns = columns_for_width(width);
                if view.visible_columns.replace(columns) != columns {
                    view.rebuild();
                }
            }

            glib::ControlFlow::Continue
        });
    }

    fn rebuild(&self) {
        // TODO: clicking an album currently scrolls the viewport back to
        // the top. `clear_container` empties the grid before re-adding
        // rows, and while the container is empty the ScrolledWindow's
        // vadjustment is clamped to 0; the new content is then laid out
        // from that position. The reveal-from-context-menu flow happens
        // to want a scroll (it explicitly re-scrolls to the selected
        // tile afterwards) but a plain tile click should keep the user
        // where they were. Fix not attempted — Claude failed on the
        // adjacent arrow-seam problem and the user pulled the trust to
        // try again here. A robust fix likely needs an incremental
        // update of the grid instead of a full clear-and-rebuild on
        // every selection change.
        self.selected_tile.replace(None);
        self.live_track_lists.borrow_mut().clear();
        clear_container(&self.container);

        let albums = self.albums.borrow().clone();
        if albums.is_empty() {
            self.container.append(&empty_albums_label());
            return;
        }

        let selected_album = self.selected_album.get();
        let columns = self.visible_columns.get().max(1);
        let mut album_index = 0;

        while album_index < albums.len() {
            let row_start = album_index;
            let row_end = (row_start + columns).min(albums.len());
            let tile_row = self.build_tile_row(
                &albums[row_start..row_end],
                row_start,
                columns,
                selected_album,
            );
            self.container.append(&tile_row);

            if let Some(selected_index) = selected_album {
                if (row_start..row_end).contains(&selected_index) {
                    let detail = self.album_detail(
                        &albums[selected_index],
                        selected_index - row_start,
                        columns,
                    );
                    self.container.append(&detail);
                }
            }

            album_index = row_end;
        }
    }

    fn build_tile_row(
        &self,
        albums: &[AlbumViewModel],
        row_start: usize,
        columns: usize,
        selected_album: Option<usize>,
    ) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, ALBUM_GRID_COLUMN_SPACING);
        row.set_homogeneous(true);
        row.set_margin_start(ALBUM_GRID_MARGIN);
        row.set_margin_end(ALBUM_GRID_MARGIN);

        for offset in 0..columns {
            if let Some(album) = albums.get(offset) {
                let index = row_start + offset;
                let tile = self.album_tile(index, album, selected_album == Some(index));
                row.append(&tile);
            } else {
                // Empty placeholder keeps later rows aligned with full-width rows.
                row.append(&gtk::Box::new(gtk::Orientation::Vertical, 0));
            }
        }

        row
    }

    fn album_tile(
        &self,
        album_index: usize,
        album: &AlbumViewModel,
        is_selected: bool,
    ) -> gtk::Button {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.set_width_request(ALBUM_TILE_WIDTH);
        content.set_halign(gtk::Align::Center);

        let artwork = self.album_artwork(album);
        content.append(&album_cover(
            artwork.texture,
            ALBUM_TILE_COVER_SIZE,
            "album-cover",
        ));

        let title = gtk::Label::new(Some(&album.title));
        title.add_css_class("album-tile-title");
        title.set_wrap(true);
        title.set_lines(2);
        title.set_xalign(0.0);
        title.set_halign(gtk::Align::Fill);
        content.append(&title);

        let artist = gtk::Label::new(Some(&album.artist));
        artist.add_css_class("album-tile-artist");
        artist.set_wrap(true);
        artist.set_lines(1);
        artist.set_xalign(0.0);
        artist.set_halign(gtk::Align::Fill);
        content.append(&artist);

        let button = gtk::Button::new();
        button.add_css_class("album-tile");
        if is_selected {
            self.selected_tile.replace(Some(button.clone()));
        }
        button.set_child(Some(&content));
        button.set_halign(gtk::Align::Fill);
        button.set_valign(gtk::Align::Start);

        let view = self.clone();
        button.connect_clicked(move |_| {
            view.selected_album.set(Some(album_index));
            view.rebuild();
        });

        button
    }

    fn album_detail(
        &self,
        album: &AlbumViewModel,
        selected_column: usize,
        columns: usize,
    ) -> gtk::Overlay {
        // Spacing here is the gap between the title-block / track-lists
        // column on the left and the artwork column on the right. Kept in
        // sync with the inter-column spacing of `lists` so the
        // right-half track column sits the same distance from the
        // artwork as the two track columns sit from each other.
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 40);
        content.add_css_class("album-detail");
        content.set_hexpand(true);
        let artwork = self.album_artwork(album);
        let palette_provider = artwork.palette.map(album_detail_palette_provider);
        install_palette_provider(&content, palette_provider.as_ref());
        apply_palette_style(
            &content,
            palette_provider.as_ref(),
            "album-detail-dominant-color",
        );

        let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
        left.set_hexpand(true);
        left.set_vexpand(true);

        let title_block = gtk::Box::new(gtk::Orientation::Vertical, 2);
        title_block.set_hexpand(true);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.set_hexpand(true);

        let title = gtk::Label::new(Some(&album.title));
        title.add_css_class("album-detail-title");
        apply_palette_style(
            &title,
            palette_provider.as_ref(),
            "album-detail-palette-primary",
        );
        title.set_xalign(0.0);
        title.set_hexpand(false);
        title.set_wrap(true);
        header.append(&title);

        let play_button = detail_icon_button(
            "media-playback-start-symbolic",
            "Play album",
            palette_provider.as_ref(),
        );
        let album_for_play = album.clone();
        let command_controller_for_play = self.command_controller.clone();
        let playback_changed_for_play = self.playback_changed.clone();
        play_button.connect_clicked(move |_| {
            if play_album(&command_controller_for_play, &album_for_play) {
                playback_changed_for_play();
            }
        });
        header.append(&play_button);

        let shuffle_button = detail_icon_button(
            "media-playlist-shuffle-symbolic",
            "Shuffle album",
            palette_provider.as_ref(),
        );
        let album_for_shuffle = album.clone();
        let command_controller_for_shuffle = self.command_controller.clone();
        let playback_changed_for_shuffle = self.playback_changed.clone();
        shuffle_button.connect_clicked(move |_| {
            ensure_shuffle_enabled(&command_controller_for_shuffle);
            if play_album(&command_controller_for_shuffle, &album_for_shuffle) {
                playback_changed_for_shuffle();
            }
        });
        header.append(&shuffle_button);

        // Trailing spacer absorbs the rest of the header row so title +
        // buttons pile up at the start instead of buttons being pushed
        // against the right edge by an hexpanding title.
        let header_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header_spacer.set_hexpand(true);
        header.append(&header_spacer);

        title_block.append(&header);

        let subtitle = gtk::Label::new(Some(&album_subtitle(album)));
        subtitle.add_css_class("album-detail-subtitle");
        apply_palette_style(
            &subtitle,
            palette_provider.as_ref(),
            "album-detail-palette-secondary",
        );
        subtitle.set_xalign(0.0);
        title_block.append(&subtitle);
        left.append(&title_block);

        let track_lists = self.album_track_lists(album, palette_provider.as_ref());
        track_lists.set_margin_top(14);
        left.append(&track_lists);

        let artwork_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        artwork_column.set_halign(gtk::Align::End);
        artwork_column.set_valign(gtk::Align::End);
        let detail_cover = album_cover(
            artwork.texture,
            ALBUM_DETAIL_ARTWORK_SIZE,
            "album-detail-cover",
        );
        apply_palette_style(
            &detail_cover,
            palette_provider.as_ref(),
            "album-detail-palette-surface",
        );
        artwork_column.append(&detail_cover);

        // Reserve vertical room above the panel for the arrow. The arrow
        // is rendered as an overlay on top of this region so its texture's
        // bottom edge overlaps the panel's top edge; any sub-pixel
        // sampling artifact in the arrow's bottom row composites over the
        // panel's opaque background (same color) instead of revealing the
        // window's theme background. This holds even when the scroller
        // translates the contents to a fractional pixel offset.
        let arrow_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        arrow_spacer.set_size_request(-1, ALBUM_DETAIL_ARROW_HEIGHT);

        let base = gtk::Box::new(gtk::Orientation::Vertical, 0);
        base.set_hexpand(true);
        base.append(&arrow_spacer);
        base.append(&content);

        let shell = gtk::Overlay::new();
        shell.set_hexpand(true);
        shell.set_child(Some(&base));

        let arrow_row = album_detail_arrow_row(selected_column, columns, palette_provider.as_ref());
        arrow_row.set_valign(gtk::Align::Start);
        arrow_row.set_can_target(false);
        shell.add_overlay(&arrow_row);

        content.append(&left);
        content.append(&artwork_column);

        shell
    }

    fn album_track_lists(
        &self,
        album: &AlbumViewModel,
        palette_provider: Option<&gtk::CssProvider>,
    ) -> gtk::Box {
        let lists = gtk::Box::new(gtk::Orientation::Horizontal, 40);
        lists.add_css_class("album-track-lists");
        lists.set_hexpand(true);

        let split_index = album.tracks.len().div_ceil(2);
        let playing_track_id = self.playing_track_id.get();

        let left = AlbumTrackListView::new(
            &album.tracks[..split_index],
            palette_provider,
            self.context_menu.clone(),
            self.command_controller.clone(),
            self.playback_changed.clone(),
            playing_track_id,
        );
        let right = AlbumTrackListView::new(
            &album.tracks[split_index..],
            palette_provider,
            self.context_menu.clone(),
            self.command_controller.clone(),
            self.playback_changed.clone(),
            playing_track_id,
        );

        let left_widget = left.widget();
        let right_widget = right.widget();
        left_widget.set_hexpand(true);
        right_widget.set_hexpand(true);

        {
            let mut live = self.live_track_lists.borrow_mut();
            live.push(left);
            live.push(right);
        }

        lists.append(&left_widget);
        lists.append(&right_widget);
        lists
    }

    fn album_artwork(&self, album: &AlbumViewModel) -> CachedArtwork {
        let Some(root) = self.runtime.borrow().settings().library.path.clone() else {
            return CachedArtwork::default();
        };
        let Some(track) = album
            .tracks
            .iter()
            .find(|track| !track.is_missing)
            .or_else(|| album.tracks.first())
        else {
            return CachedArtwork::default();
        };
        let artwork_path = if track.file_path.is_absolute() {
            track.file_path.clone()
        } else {
            root.join(&track.file_path)
        };

        if let Some(cached) = self.artwork_cache.borrow().get(&artwork_path) {
            return cached.clone();
        }

        let artwork = self
            .runtime
            .borrow()
            .read_artwork(&artwork_path)
            .and_then(artwork_from_bytes)
            .unwrap_or_default();
        self.artwork_cache
            .borrow_mut()
            .insert(artwork_path, artwork.clone());
        artwork
    }
}

fn clear_container(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn columns_for_width(width: i32) -> usize {
    let usable_width = width
        .saturating_sub(ALBUM_GRID_MARGIN * 2)
        .max(ALBUM_TILE_MIN_WIDTH);
    ((usable_width + ALBUM_GRID_COLUMN_SPACING)
        / (ALBUM_TILE_MIN_WIDTH + ALBUM_GRID_COLUMN_SPACING))
        .max(1) as usize
}

fn empty_albums_label() -> gtk::Label {
    let label = gtk::Label::new(Some("No albums"));
    label.add_css_class("album-empty-state");
    label.set_margin_top(24);
    label.set_margin_end(24);
    label.set_margin_bottom(24);
    label.set_margin_start(24);
    label
}

fn album_cover(texture: Option<gdk::Texture>, size: i32, css_class: &str) -> gtk::Box {
    let cover = gtk::Box::new(gtk::Orientation::Vertical, 0);
    cover.add_css_class(css_class);
    cover.set_size_request(size, size);
    cover.set_halign(gtk::Align::Center);
    cover.set_valign(gtk::Align::Center);
    cover.set_hexpand(false);
    cover.set_vexpand(false);
    cover.set_overflow(gtk::Overflow::Hidden);

    match texture {
        Some(texture) => {
            // gtk::Image with set_pixel_size renders at exactly `size`, unlike
            // gtk::Picture whose natural size matches the texture's intrinsic
            // dimensions and inflates the cover's parent allocation.
            let image = gtk::Image::from_paintable(Some(&texture));
            image.set_pixel_size(size);
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            image.set_hexpand(true);
            image.set_vexpand(true);
            cover.append(&image);
        }
        None => {
            if let Some(icon) = album_cover_placeholder(size) {
                cover.append(&icon);
            }
        }
    }

    cover
}

fn album_cover_placeholder(size: i32) -> Option<gtk::Image> {
    let display = gtk::gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    if !theme.has_icon(ALBUM_COVER_PLACEHOLDER_ICON) {
        return None;
    }

    let icon = gtk::Image::from_icon_name(ALBUM_COVER_PLACEHOLDER_ICON);
    icon.add_css_class("album-cover-placeholder-icon");
    icon.set_pixel_size((size / 3).max(32));
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_hexpand(true);
    icon.set_vexpand(true);
    Some(icon)
}

fn album_detail_arrow_row(
    selected_column: usize,
    columns: usize,
    palette_provider: Option<&gtk::CssProvider>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, ALBUM_GRID_COLUMN_SPACING);
    row.set_homogeneous(true);
    row.set_margin_start(ALBUM_GRID_MARGIN);
    row.set_margin_end(ALBUM_GRID_MARGIN);
    row.set_height_request(ALBUM_DETAIL_ARROW_HEIGHT + ALBUM_DETAIL_ARROW_BLEED);

    for column in 0..columns {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        cell.set_halign(gtk::Align::Fill);
        cell.set_hexpand(true);

        if column == selected_column {
            cell.append(&album_detail_arrow(palette_provider));
        }

        row.append(&cell);
    }

    row
}

fn album_detail_arrow(palette_provider: Option<&gtk::CssProvider>) -> gtk::DrawingArea {
    let arrow = gtk::DrawingArea::new();
    arrow.add_css_class("album-detail-arrow");
    apply_palette_style(&arrow, palette_provider, "album-detail-palette-arrow");
    arrow.set_content_width(ALBUM_DETAIL_ARROW_WIDTH);
    arrow.set_content_height(ALBUM_DETAIL_ARROW_HEIGHT + ALBUM_DETAIL_ARROW_BLEED);
    arrow.set_halign(gtk::Align::Center);
    arrow.set_valign(gtk::Align::End);
    arrow.set_draw_func(|area, context, width, _height| {
        let color = area.color();
        let arrow_width = f64::from(width);
        let arrow_height = f64::from(ALBUM_DETAIL_ARROW_HEIGHT);
        let bleed = f64::from(ALBUM_DETAIL_ARROW_BLEED);

        // The arrow color is driven by CSS so it stays in sync with the
        // panel: `.album-detail-arrow` matches the default panel tint, and
        // `.album-detail-palette-arrow` (applied when artwork yields a
        // palette) matches the palette background. Alpha is forced to 1.0
        // so the panel's 1px overlap below can't composite onto a
        // translucent fill and produce a darker stripe at the seam.
        context.set_source_rgba(
            f64::from(color.red()),
            f64::from(color.green()),
            f64::from(color.blue()),
            1.0,
        );

        context.move_to(arrow_width / 2.0, 0.0);
        context.line_to(arrow_width, arrow_height);
        context.line_to(0.0, arrow_height);
        context.close_path();
        let _result = context.fill();

        context.rectangle(0.0, arrow_height, arrow_width, bleed);
        let _result = context.fill();
    });

    arrow
}

fn album_detail_palette_provider(palette: ArtworkPalette) -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(&album_detail_palette_css(palette));
    provider
}

fn apply_palette_style(
    widget: &impl IsA<gtk::Widget>,
    provider: Option<&gtk::CssProvider>,
    css_class: &str,
) {
    if provider.is_none() {
        return;
    }

    widget.as_ref().add_css_class(css_class);
}

fn install_palette_provider(widget: &impl IsA<gtk::Widget>, provider: Option<&gtk::CssProvider>) {
    let (Some(display), Some(provider)) = (gdk::Display::default(), provider) else {
        return;
    };

    gtk::style_context_add_provider_for_display(
        &display,
        provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
    );

    let provider = provider.clone();
    widget.as_ref().connect_destroy(move |_| {
        gtk::style_context_remove_provider_for_display(&display, &provider);
    });
}

fn album_detail_palette_css(palette: ArtworkPalette) -> String {
    let background = palette.background_css();
    let foreground = palette.foreground_css();

    format!(
        r#"
        .album-detail-dominant-color {{
            background-color: {background};
            border: none;
            color: {foreground};
        }}

        .album-detail-palette-arrow {{
            color: {background};
        }}

        .album-detail-palette-primary,
        button.album-detail-palette-button,
        image.album-detail-palette-primary {{
            color: {foreground};
        }}

        .album-detail-palette-secondary {{
            color: alpha({foreground}, 0.78);
        }}

        .album-detail-palette-muted {{
            color: alpha({foreground}, 0.62);
        }}

        .album-detail-palette-surface {{
            background-color: alpha({foreground}, 0.12);
        }}

        button.album-detail-palette-button:hover,
        button.album-detail-palette-button:active,
        button.album-detail-palette-button:focus {{
            background-color: alpha({foreground}, 0.14);
        }}

        .album-track-table .track-table-status-playing {{
            color: {foreground};
        }}

        listview.album-track-table > row:focus-visible {{
            outline-color: {foreground};
        }}
        "#,
    )
}

fn detail_icon_button(
    icon_name: &str,
    tooltip: &str,
    palette_provider: Option<&gtk::CssProvider>,
) -> gtk::Button {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(18);
    apply_palette_style(&icon, palette_provider, "album-detail-palette-primary");

    let button = gtk::Button::new();
    button.add_css_class("album-detail-icon-button");
    apply_palette_style(&button, palette_provider, "album-detail-palette-button");
    button.set_child(Some(&icon));
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button
}

fn play_album(command_controller: &SharedCommandController, album: &AlbumViewModel) -> bool {
    let Some(track_id) = album
        .tracks
        .iter()
        .find(|track| !track.is_missing)
        .map(|track| track.id)
    else {
        return false;
    };

    command_controller.dispatch_succeeded(ApplicationCommand::Playback(PlaybackCommand::PlayTrack(
        track_id,
    )))
}

fn ensure_shuffle_enabled(command_controller: &SharedCommandController) {
    if command_controller
        .runtime()
        .borrow()
        .playback_options()
        .shuffle_enabled
    {
        return;
    }

    let _result =
        command_controller.dispatch(ApplicationCommand::Playback(PlaybackCommand::ToggleShuffle));
}

fn artwork_from_bytes(bytes: Vec<u8>) -> Option<CachedArtwork> {
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes)).ok()?;
    Some(CachedArtwork {
        texture: Some(gdk::Texture::for_pixbuf(&pixbuf)),
        palette: ArtworkPalette::from_pixbuf(&pixbuf),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ALBUM_GRID_COLUMN_SPACING, ALBUM_GRID_MARGIN, ALBUM_TILE_MIN_WIDTH, columns_for_width,
    };

    #[test]
    fn columns_follow_available_width() {
        assert_eq!(columns_for_width(120), 1);
        assert_eq!(columns_for_width(520), 2);
        assert_eq!(columns_for_width(1200), 6);
        assert_eq!(columns_for_width(2400), 13);
    }

    #[test]
    fn columns_account_for_spacing_between_tiles() {
        let two_column_width =
            ALBUM_GRID_MARGIN * 2 + ALBUM_TILE_MIN_WIDTH * 2 + ALBUM_GRID_COLUMN_SPACING;

        assert_eq!(columns_for_width(two_column_width - 1), 1);
        assert_eq!(columns_for_width(two_column_width), 2);
    }
}
