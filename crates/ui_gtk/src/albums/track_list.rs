// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::{cell::RefCell, rc::Rc};
use sustain_app_runtime::{ApplicationCommand, PlaybackCommand, TrackId};

use super::model::{AlbumTrackViewModel, duration_text, track_number_text};
use crate::{
    PlaybackChangedCallback, command_controller::SharedCommandController,
    track_context::TrackRowContextMenu,
};

const STATUS_ICON_SIZE: i32 = 14;
const NUMBER_OR_STATUS_CELL_WIDTH: i32 = 24;
const TRACK_NUMBER_MIN_CHARS: i32 = 2;
const TRACK_TITLE_MAX_CHARS: i32 = 56;
const STATUS_ICON_PLAYING: &str = "audio-volume-high-symbolic";
const STATUS_ICON_MISSING: &str = "dialog-warning-symbolic";

#[derive(Clone)]
pub(super) struct AlbumTrackListView {
    widget: gtk::Overlay,
}

impl AlbumTrackListView {
    pub(super) fn new(
        tracks: &[AlbumTrackViewModel],
        palette_provider: Option<&gtk::CssProvider>,
        context_menu: TrackRowContextMenu,
        command_controller: SharedCommandController,
        playback_changed: PlaybackChangedCallback,
        playing_track_id: Option<TrackId>,
    ) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        for track in tracks {
            store.append(&glib::BoxedAnyObject::new(track.clone()));
        }
        // Albums-view track rows are not selectable: nothing in the UI acts on
        // an album-view selection, so a persistent highlight would be visual
        // noise that lies about state. Activation (double-click / Enter) and
        // right-click context menus still work without a selection model.
        let selection = gtk::NoSelection::new(Some(store));

        let popover_anchor = build_context_popover_anchor();
        let context_menu = AlbumTrackContextMenu::new(context_menu, popover_anchor.clone());
        let factory = build_row_factory(
            palette_provider.cloned(),
            context_menu.clone(),
            playing_track_id,
        );

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("album-track-table");
        list.set_show_separators(false);
        list.set_single_click_activate(false);
        list.set_hexpand(true);
        list.set_vexpand(false);
        context_menu.install_controller(&list);

        let command_controller_for_activate = command_controller;
        let playback_changed_for_activate = playback_changed;
        list.connect_activate(move |_list, position| {
            let Some(track_id) = row_track_id(selection.item(position)) else {
                return;
            };
            if command_controller_for_activate.dispatch_succeeded(ApplicationCommand::Playback(
                PlaybackCommand::PlayTrack(track_id),
            )) {
                playback_changed_for_activate();
            }
        });

        // `GtkListView` implements `GtkScrollable` and expects to live inside a
        // `GtkScrolledWindow`. Wrap it in a non-scrolling one with
        // `propagate-natural-height` so the list requests the full height of
        // its rows and the outer Albums-view scroller handles overflow.
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        scroller.set_propagate_natural_height(true);
        scroller.set_propagate_natural_width(false);
        scroller.set_hexpand(true);
        scroller.set_vexpand(false);
        scroller.add_css_class("album-track-table-scroller");
        scroller.set_child(Some(&list));

        let shell = gtk::Overlay::new();
        shell.set_hexpand(true);
        shell.set_vexpand(false);
        shell.set_child(Some(&scroller));
        shell.add_overlay(&popover_anchor);

        Self { widget: shell }
    }

    pub(super) fn widget(&self) -> gtk::Overlay {
        self.widget.clone()
    }
}

fn build_row_factory(
    palette_provider: Option<gtk::CssProvider>,
    context_menu: AlbumTrackContextMenu,
    playing_track_id: Option<TrackId>,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let palette_present = palette_provider.is_some();
    drop(palette_provider);

    let context_for_setup = context_menu;
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("album-track-row");
        row.set_hexpand(true);

        // First cell: track number, replaced by speaker (playing) or
        // warning (missing) icon when one of those states applies. The
        // icon and the number live in the same Box; visibility is toggled
        // in `refresh_status_icon` so only one is shown at a time and the
        // cell width stays stable.
        let number_or_status = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        number_or_status.add_css_class("album-track-number-cell");
        number_or_status.set_halign(gtk::Align::End);
        number_or_status.set_valign(gtk::Align::Center);
        number_or_status.set_size_request(NUMBER_OR_STATUS_CELL_WIDTH, -1);

        let status_icon = gtk::Image::new();
        status_icon.set_pixel_size(STATUS_ICON_SIZE);
        status_icon.set_halign(gtk::Align::End);
        status_icon.set_valign(gtk::Align::Center);
        status_icon.add_css_class("track-table-status-icon");
        status_icon.set_visible(false);
        number_or_status.append(&status_icon);

        let number = gtk::Label::new(None);
        number.add_css_class("album-track-number");
        number.set_xalign(1.0);
        number.set_width_chars(TRACK_NUMBER_MIN_CHARS);
        if palette_present {
            number.add_css_class("album-detail-palette-muted");
        }
        number_or_status.append(&number);

        row.append(&number_or_status);

        let title = gtk::Label::new(None);
        title.add_css_class("album-track-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_width_chars(1);
        title.set_max_width_chars(TRACK_TITLE_MAX_CHARS);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        if palette_present {
            title.add_css_class("album-detail-palette-primary");
        }
        row.append(&title);

        let duration = gtk::Label::new(None);
        duration.add_css_class("album-track-duration");
        duration.set_xalign(1.0);
        if palette_present {
            duration.add_css_class("album-detail-palette-muted");
        }
        row.append(&duration);

        context_for_setup.register_row(list_item, &row);

        list_item.set_child(Some(&row));
    });

    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        let Some((status_icon, number, title, duration)) = row_widgets(&row) else {
            return;
        };
        let Some(row_object) = list_item
            .item()
            .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
        else {
            return;
        };
        let Ok(track) = row_object.try_borrow::<AlbumTrackViewModel>() else {
            return;
        };

        number.set_text(&track_number_text(&track));
        title.set_text(&track.title);
        duration.set_text(&duration_text(track.duration_seconds));

        sync_missing_class(&title, track.is_missing);
        drop(track);

        refresh_status_icon(list_item, &status_icon, &number, playing_track_id);
    });

    factory
}

fn row_widgets(row: &gtk::Box) -> Option<(gtk::Image, gtk::Label, gtk::Label, gtk::Label)> {
    let number_or_status = row.first_child()?.downcast::<gtk::Box>().ok()?;
    let status = number_or_status
        .first_child()?
        .downcast::<gtk::Image>()
        .ok()?;
    let number = status.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let title = number_or_status
        .next_sibling()?
        .downcast::<gtk::Label>()
        .ok()?;
    let duration = title.next_sibling()?.downcast::<gtk::Label>().ok()?;
    Some((status, number, title, duration))
}

fn sync_missing_class(title: &gtk::Label, is_missing: bool) {
    if is_missing {
        title.add_css_class("album-track-missing");
    } else {
        title.remove_css_class("album-track-missing");
    }
}

fn row_track_id(item: Option<glib::Object>) -> Option<TrackId> {
    let row_object = item?.downcast::<glib::BoxedAnyObject>().ok()?;
    let track = row_object.try_borrow::<AlbumTrackViewModel>().ok()?;
    Some(track.id)
}

#[derive(Clone)]
struct AlbumTrackContextMenu {
    menu: TrackRowContextMenu,
    popover_anchor: gtk::MenuButton,
    rows: Rc<RefCell<Vec<AlbumTrackContextRow>>>,
}

impl AlbumTrackContextMenu {
    fn new(menu: TrackRowContextMenu, popover_anchor: gtk::MenuButton) -> Self {
        Self {
            menu,
            popover_anchor,
            rows: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn install_controller(&self, widget: &impl IsA<gtk::Widget>) {
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gdk::BUTTON_SECONDARY);
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);

        let context = self.clone();
        gesture.connect_released(move |gesture, _n_press, x, y| {
            let Some(widget) = gesture.widget() else {
                return;
            };
            let Some(hit) = context.row_at(&widget, x, y) else {
                return;
            };
            let Some(track_id) = row_track_id(hit.list_item.item()) else {
                return;
            };

            gesture.set_state(gtk::EventSequenceState::Claimed);
            let (anchor_x, anchor_y) = widget
                .compute_point(
                    &context.popover_anchor,
                    &gtk::graphene::Point::new(x as f32, y as f32),
                )
                .map(|point| (point.x() as f64, point.y() as f64))
                .unwrap_or((x, y));
            let menu = context.menu.clone();
            let popover_anchor = context.popover_anchor.clone();
            glib::idle_add_local_once(move || {
                menu.popup_from_menu_button(vec![track_id], &popover_anchor, anchor_x, anchor_y);
            });
        });
        widget.add_controller(gesture);
    }

    fn register_row(&self, list_item: &gtk::ListItem, row: &gtk::Box) {
        self.rows.borrow_mut().push(AlbumTrackContextRow {
            widget: row.clone().upcast(),
            list_item: list_item.clone(),
        });
    }

    fn row_at(&self, event_widget: &gtk::Widget, x: f64, y: f64) -> Option<AlbumTrackContextRow> {
        let mut current = event_widget.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(widget) = current {
            if let Some(hit) = self
                .rows
                .borrow()
                .iter()
                .find(|row| row.widget == widget)
                .cloned()
            {
                return Some(hit);
            }
            current = widget.parent();
        }
        None
    }
}

fn build_context_popover_anchor() -> gtk::MenuButton {
    let anchor = gtk::MenuButton::new();
    anchor.add_css_class("context-popover-anchor");
    anchor.set_has_frame(false);
    anchor.set_focusable(false);
    anchor.set_can_target(false);
    anchor.set_halign(gtk::Align::Fill);
    anchor.set_valign(gtk::Align::Fill);
    anchor.set_hexpand(true);
    anchor.set_vexpand(true);
    anchor
}

#[derive(Clone)]
struct AlbumTrackContextRow {
    widget: gtk::Widget,
    list_item: gtk::ListItem,
}

fn refresh_status_icon(
    list_item: &gtk::ListItem,
    icon: &gtk::Image,
    number: &gtk::Label,
    playing_track_id: Option<TrackId>,
) {
    let Some(row_object) = list_item
        .item()
        .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
    else {
        clear_status_icon(icon, number);
        return;
    };
    let Ok(track) = row_object.try_borrow::<AlbumTrackViewModel>() else {
        clear_status_icon(icon, number);
        return;
    };

    icon.remove_css_class("track-table-status-playing");
    icon.remove_css_class("track-table-status-missing");

    if track.is_missing {
        icon.set_icon_name(Some(STATUS_ICON_MISSING));
        icon.add_css_class("track-table-status-missing");
        icon.set_visible(true);
        number.set_visible(false);
        return;
    }

    if matches!(playing_track_id, Some(playing_id) if track.id == playing_id) {
        icon.set_icon_name(Some(STATUS_ICON_PLAYING));
        icon.add_css_class("track-table-status-playing");
        icon.set_visible(true);
        number.set_visible(false);
        return;
    }

    clear_status_icon(icon, number);
}

fn clear_status_icon(icon: &gtk::Image, number: &gtk::Label) {
    icon.set_icon_name(None);
    icon.set_visible(false);
    number.set_visible(true);
}
