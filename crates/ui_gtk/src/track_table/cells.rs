// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;
use gtk::{gdk, glib, graphene};
use sustain_app_runtime::{Rating, TrackId};

use super::{RatingChangedCallback, columns::TrackTableColumn, row::TrackTableRow};
use crate::sidebar::tracks_drag_payload;
use crate::track_context::TrackRowContextMenu;

const EMPTY_STAR: &str = "☆";
const FILLED_STAR: &str = "★";
const MAX_RATING: u8 = 5;
const STATUS_COLUMN_WIDTH: i32 = 26;
const STATUS_ICON_SIZE: i32 = 14;
const STATUS_ICON_PLAYING: &str = "audio-volume-high-symbolic";
const STATUS_ICON_MISSING: &str = "dialog-warning-symbolic";

#[derive(Clone)]
pub(super) struct TrackTableContextMenu {
    menu: TrackRowContextMenu,
    selection: gtk::MultiSelection,
    popover_parent: gtk::ScrolledWindow,
}

impl TrackTableContextMenu {
    pub(super) fn new(
        menu: TrackRowContextMenu,
        selection: gtk::MultiSelection,
        popover_parent: gtk::ScrolledWindow,
    ) -> Self {
        Self {
            menu,
            selection,
            popover_parent,
        }
    }
}

struct StatusBinding {
    list_item: gtk::ListItem,
    icon: gtk::Image,
}

#[derive(Clone, Default)]
pub(super) struct StatusBindings(Rc<RefCell<Vec<StatusBinding>>>);

impl StatusBindings {
    pub(super) fn refresh(&self, playing_track_id: Option<TrackId>) {
        for binding in self.0.borrow().iter() {
            refresh_status_icon(&binding.list_item, &binding.icon, playing_track_id);
        }
    }
}

pub(super) fn build_status_column(
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    bindings: StatusBindings,
    context_menu: Option<TrackTableContextMenu>,
) -> gtk::ColumnViewColumn {
    let factory = build_status_cell_factory(playing_track_id, bindings, context_menu);
    let table_column = gtk::ColumnViewColumn::new(None, Some(factory));
    table_column.set_resizable(false);
    table_column.set_fixed_width(STATUS_COLUMN_WIDTH);
    table_column.set_visible(true);
    table_column
}

pub(super) fn build_text_cell_factory(
    column: TrackTableColumn,
    context_menu: Option<TrackTableContextMenu>,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let context_for_setup = context_menu;
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_chrome(list_item, &cell, context_for_setup.as_ref());

        let label = gtk::Label::new(None);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_hexpand(true);
        label.set_valign(gtk::Align::Center);
        label.set_margin_start(8);
        label.set_margin_end(8);
        label.set_xalign(column.xalign());

        cell.append(&label);
        list_item.set_child(Some(&cell));
    });

    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());

        let Some(label) = cell
            .first_child()
            .and_then(|child| child.downcast::<gtk::Label>().ok())
        else {
            return;
        };
        let Some(row_object) = list_item
            .item()
            .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
        else {
            return;
        };
        let Ok(row) = row_object.try_borrow::<TrackTableRow>() else {
            return;
        };

        label.set_text(&column.text(&row));
    });

    factory
}

pub(super) fn build_rating_cell_factory(
    context_menu: Option<TrackTableContextMenu>,
    rating_changed: Option<RatingChangedCallback>,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let context_for_setup = context_menu;
    let rating_changed_for_bind = rating_changed;
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_chrome(list_item, &cell, context_for_setup.as_ref());
        list_item.set_child(Some(&cell));
    });

    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());
        clear_box_children(&cell);

        let Some(row_object) = list_item
            .item()
            .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
        else {
            return;
        };
        let Ok(row) = row_object.try_borrow::<TrackTableRow>() else {
            return;
        };
        let rating = row.rating;
        drop(row);

        let rating_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        rating_box.add_css_class("rating-stars");
        rating_box.set_margin_start(6);
        rating_box.set_margin_end(6);
        rating_box.set_halign(gtk::Align::End);
        rating_box.set_valign(gtk::Align::Center);

        for star in 1..=MAX_RATING {
            let button = gtk::Button::with_label("");
            button.add_css_class("flat");
            button.add_css_class("rating-star");
            sync_rating_button(&button, star, rating);

            let row_object_for_click = row_object.clone();
            let rating_box_for_click = rating_box.clone();
            let rating_changed_for_click = rating_changed_for_bind.clone();
            button.connect_clicked(move |_| {
                let Ok(row) = row_object_for_click.try_borrow::<TrackTableRow>() else {
                    return;
                };
                let Some(track_id) = row.track_id else {
                    return;
                };
                let new_rating = rating_after_click(row.rating, star);
                drop(row);

                let Some(rating) = Rating::new(new_rating) else {
                    return;
                };
                let Some(rating_changed) = rating_changed_for_click.as_ref() else {
                    return;
                };

                if rating_changed(track_id, rating) {
                    sync_rating_buttons(&rating_box_for_click, new_rating);
                }
            });

            rating_box.append(&button);
        }

        cell.append(&rating_box);
    });

    factory
}

pub(super) fn build_filler_column(
    context_menu: Option<TrackTableContextMenu>,
) -> gtk::ColumnViewColumn {
    let table_column = gtk::ColumnViewColumn::new(None, Some(build_filler_factory(context_menu)));
    table_column.set_expand(true);
    table_column.set_resizable(false);
    table_column.set_visible(true);
    table_column
}

fn build_status_cell_factory(
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    bindings: StatusBindings,
    context_menu: Option<TrackTableContextMenu>,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();

    let bindings_for_setup = bindings.clone();
    let context_for_setup = context_menu;
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_chrome(list_item, &cell, context_for_setup.as_ref());

        let icon = gtk::Image::new();
        icon.set_pixel_size(STATUS_ICON_SIZE);
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        icon.set_hexpand(true);
        icon.add_css_class("track-table-status-icon");
        cell.append(&icon);

        list_item.set_child(Some(&cell));

        bindings_for_setup.0.borrow_mut().push(StatusBinding {
            list_item: list_item.clone(),
            icon,
        });
    });

    let bindings_for_teardown = bindings;
    factory.connect_teardown(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        bindings_for_teardown
            .0
            .borrow_mut()
            .retain(|binding| binding.list_item != *list_item);
    });

    let playing_for_bind = playing_track_id;
    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());

        let Some(icon) = cell
            .first_child()
            .and_then(|child| child.downcast::<gtk::Image>().ok())
        else {
            return;
        };
        refresh_status_icon(list_item, &icon, playing_for_bind.get());
    });

    factory
}

fn build_filler_factory(context_menu: Option<TrackTableContextMenu>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let context_for_setup = context_menu;
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_chrome(list_item, &cell, context_for_setup.as_ref());
        list_item.set_child(Some(&cell));
    });

    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());
    });

    factory
}

fn install_cell_chrome(
    list_item: &gtk::ListItem,
    cell: &gtk::Box,
    context_menu: Option<&TrackTableContextMenu>,
) {
    install_cell_selection_sync(list_item, cell);
    if let Some(menu) = context_menu {
        install_cell_context_menu(list_item, cell, menu);
        install_cell_drag_source(list_item, cell, &menu.selection);
    }
}

fn install_cell_drag_source(
    list_item: &gtk::ListItem,
    cell: &gtk::Box,
    selection: &gtk::MultiSelection,
) {
    let drag_source = gtk::DragSource::new();
    drag_source.set_actions(gdk::DragAction::COPY);

    let list_item = list_item.clone();
    let selection = selection.clone();
    let cell_for_prepare = cell.clone();
    drag_source.connect_prepare(move |source, _x, _y| {
        let position = list_item.position();
        let row_track_id = row_track_id(list_item.item())?;

        let track_ids = if position != gtk::INVALID_LIST_POSITION && selection.is_selected(position)
        {
            let mut selected = collect_selected_track_ids(&selection);
            if !selected.contains(&row_track_id) {
                selected.push(row_track_id);
            }
            selected
        } else {
            vec![row_track_id]
        };

        if track_ids.is_empty() {
            return None;
        }

        if let Some(paintable) = build_drag_paintable(&cell_for_prepare, position, &selection) {
            source.set_icon(Some(&paintable), 0, 0);
        }

        Some(gdk::ContentProvider::for_value(
            &tracks_drag_payload(&track_ids).to_value(),
        ))
    });
    cell.add_controller(drag_source);
}

/// Build the drag image. Single-track drags use a [`gtk::WidgetPaintable`] of
/// the originating row so the row image follows the cursor. Multi-track drags
/// composite the visible selected rows into a stacked snapshot via
/// [`gtk::Snapshot::to_paintable`].
///
/// The multi-row composite leans on three implicit GTK4 invariants. If any of
/// them ever stops holding, the originating row's plain [`gtk::WidgetPaintable`]
/// is returned as a graceful fallback (the icon will still follow the cursor;
/// it just won't show the stack).
///
/// 1. [`find_listview_row`] assumes the ColumnView's row container has the CSS
///    node name `row`. Stable in GTK4 today but not a contract — if a future
///    GTK reworks the listview hierarchy, `find_listview_row` would return
///    `None` and we fall straight back to a missing-icon `None` here.
/// 2. [`visible_selected_row_widgets`] assumes sibling order in the row
///    container matches position order. True for `ListBase` virtualization in
///    GTK4, but there is no public per-widget position API to verify it; if
///    GTK ever recycles widgets out of order, we may stack the wrong rows.
/// 3. The composite calls [`gdk::Paintable::snapshot`] on a
///    [`gtk::WidgetPaintable`] wrapping a widget still parented inside the
///    live listview. WidgetPaintable is designed for this, but if a future
///    GTK refuses to paint widgets mid-virtualization, the composite returns
///    `None` and we fall back to the single-row paintable below.
fn build_drag_paintable(
    cell: &gtk::Box,
    originating_position: u32,
    selection: &gtk::MultiSelection,
) -> Option<gdk::Paintable> {
    let origin_row = find_listview_row(cell)?;

    if originating_position == gtk::INVALID_LIST_POSITION
        || !selection.is_selected(originating_position)
    {
        return Some(gtk::WidgetPaintable::new(Some(&origin_row)).upcast());
    }

    let selected_rows = visible_selected_row_widgets(&origin_row, originating_position, selection);

    if selected_rows.len() <= 1 {
        return Some(gtk::WidgetPaintable::new(Some(&origin_row)).upcast());
    }

    compose_stacked_row_paintable(&selected_rows)
        .or_else(|| Some(gtk::WidgetPaintable::new(Some(&origin_row)).upcast()))
}

/// Walk up the cell's parent chain to the ColumnView row container.
///
/// Risk: this depends on GTK4's convention that the row container has CSS
/// node name `row`. If a future GTK renames or restructures the listview
/// hierarchy this returns `None` and the drag falls back to no icon (cursor
/// without preview). Caller is responsible for the fallback.
fn find_listview_row(cell: &gtk::Box) -> Option<gtk::Widget> {
    let mut current: Option<gtk::Widget> = cell.parent();
    while let Some(widget) = current {
        if widget.css_name() == "row" {
            return Some(widget);
        }
        current = widget.parent();
    }
    None
}

/// Walk the row container's children in both directions from `origin`, gathering
/// row widgets whose positions belong to the current selection. Sibling order
/// matches position order in GTK4 `ListBase`, so we infer each sibling's position
/// from its offset relative to the originating row instead of asking the
/// widget — there's no public per-widget position API today.
///
/// Risk: if GTK ever recycles row widgets out of position order (e.g. as part
/// of an aggressive virtualization rework), our computed positions would be
/// wrong and we'd stack the wrong rows in the drag icon. The drag payload
/// (which is built independently from the selection model) is unaffected.
fn visible_selected_row_widgets(
    origin: &gtk::Widget,
    origin_position: u32,
    selection: &gtk::MultiSelection,
) -> Vec<gtk::Widget> {
    let mut collected: Vec<(u32, gtk::Widget)> = vec![(origin_position, origin.clone())];

    let mut position = origin_position;
    let mut current = origin.next_sibling();
    while let Some(sibling) = current {
        position = position.saturating_add(1);
        if selection.is_selected(position) {
            collected.push((position, sibling.clone()));
        }
        current = sibling.next_sibling();
    }

    let mut position = origin_position;
    let mut current = origin.prev_sibling();
    while let Some(sibling) = current {
        if position == 0 {
            break;
        }
        position -= 1;
        if selection.is_selected(position) {
            collected.push((position, sibling.clone()));
        }
        current = sibling.prev_sibling();
    }

    collected.sort_by_key(|(p, _)| *p);
    collected.into_iter().map(|(_, widget)| widget).collect()
}

/// Compose `rows` into a single vertically stacked paintable.
///
/// Risk: each row is painted via a [`gtk::WidgetPaintable`] that still
/// references the live row widget inside the listview. WidgetPaintable is
/// designed for exactly this use, but if a future GTK refuses to paint
/// widgets mid-virtualization the call quietly produces a blank or `None`
/// paintable. `build_drag_paintable` falls back to the originating row's
/// single paintable in that case, so the drag still shows *something*.
///
/// Returns `None` if any dimension is zero (rows weren't laid out yet) so
/// the caller can fall back instead of producing an invalid icon.
fn compose_stacked_row_paintable(rows: &[gtk::Widget]) -> Option<gdk::Paintable> {
    let widths: Vec<f32> = rows.iter().map(|row| row.width() as f32).collect();
    let heights: Vec<f32> = rows.iter().map(|row| row.height() as f32).collect();
    let total_width = widths.iter().copied().fold(0.0_f32, f32::max);
    let total_height: f32 = heights.iter().sum();
    if total_width <= 0.0 || total_height <= 0.0 {
        return None;
    }

    let snapshot = gtk::Snapshot::new();
    let mut y_offset = 0.0_f32;
    for (row, height) in rows.iter().zip(heights.iter().copied()) {
        let width = row.width() as f64;
        let paintable = gtk::WidgetPaintable::new(Some(row));
        snapshot.translate(&graphene::Point::new(0.0, y_offset));
        paintable.snapshot(snapshot.upcast_ref::<gdk::Snapshot>(), width, height as f64);
        snapshot.translate(&graphene::Point::new(0.0, -y_offset));
        y_offset += height;
    }

    snapshot.to_paintable(Some(&graphene::Size::new(total_width, total_height)))
}

fn install_cell_context_menu(
    list_item: &gtk::ListItem,
    cell: &gtk::Box,
    context: &TrackTableContextMenu,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);

    let context = context.clone();
    let list_item = list_item.clone();
    let cell_for_gesture = cell.clone();
    gesture.connect_pressed(move |_gesture, _n_press, x, y| {
        let position = list_item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        if !context.selection.is_selected(position) {
            context.selection.select_item(position, true);
        }

        let track_ids = collect_selected_track_ids(&context.selection);
        if track_ids.is_empty() {
            return;
        }
        context
            .menu
            .popup_at_parent(track_ids, &cell_for_gesture, &context.popover_parent, x, y);
    });
    cell.add_controller(gesture);
}

fn collect_selected_track_ids(selection: &gtk::MultiSelection) -> Vec<TrackId> {
    let bitset = selection.selection();
    let Some((iter, first)) = gtk::BitsetIter::init_first(&bitset) else {
        return Vec::new();
    };

    std::iter::once(first)
        .chain(iter)
        .filter_map(|position| row_track_id(selection.item(position)))
        .collect()
}

fn row_track_id(item: Option<glib::Object>) -> Option<TrackId> {
    let row_object = item?.downcast::<glib::BoxedAnyObject>().ok()?;
    let row = row_object.try_borrow::<TrackTableRow>().ok()?;
    row.track_id
}

fn refresh_status_icon(
    list_item: &gtk::ListItem,
    icon: &gtk::Image,
    playing_track_id: Option<TrackId>,
) {
    let Some(row_object) = list_item
        .item()
        .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
    else {
        clear_status_icon(icon);
        return;
    };
    let Ok(row) = row_object.try_borrow::<TrackTableRow>() else {
        clear_status_icon(icon);
        return;
    };

    icon.remove_css_class("track-table-status-playing");
    icon.remove_css_class("track-table-status-missing");

    if row.is_missing {
        icon.set_icon_name(Some(STATUS_ICON_MISSING));
        icon.add_css_class("track-table-status-missing");
        icon.set_visible(true);
        return;
    }

    if matches!(
        (row.track_id, playing_track_id),
        (Some(track_id), Some(playing_id)) if track_id == playing_id
    ) {
        icon.set_icon_name(Some(STATUS_ICON_PLAYING));
        icon.add_css_class("track-table-status-playing");
        icon.set_visible(true);
        return;
    }

    clear_status_icon(icon);
}

fn clear_status_icon(icon: &gtk::Image) {
    icon.set_icon_name(None);
    icon.set_visible(false);
}

fn apply_row_tint(cell: &gtk::Box, row_position: u32) {
    cell.remove_css_class("track-table-row-even");
    cell.remove_css_class("track-table-row-odd");
    if row_position % 2 == 0 {
        cell.add_css_class("track-table-row-even");
    } else {
        cell.add_css_class("track-table-row-odd");
    }
}

fn install_cell_selection_sync(list_item: &gtk::ListItem, cell: &gtk::Box) {
    let cell_for_selection = cell.clone();
    list_item.connect_selected_notify(move |list_item| {
        sync_row_selection_class(&cell_for_selection, list_item.is_selected());
    });
    sync_row_selection_class(cell, list_item.is_selected());
}

fn sync_row_selection_class(cell: &gtk::Box, selected: bool) {
    if selected {
        cell.add_css_class("track-table-row-selected");
    } else {
        cell.remove_css_class("track-table-row-selected");
    }
}

fn clear_box_children(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn sync_rating_buttons(rating_box: &gtk::Box, rating: u8) {
    let mut star = 1;
    let mut child = rating_box.first_child();
    while let Some(widget) = child {
        let next_child = widget.next_sibling();
        if let Ok(button) = widget.downcast::<gtk::Button>() {
            sync_rating_button(&button, star, rating);
            star += 1;
        }
        child = next_child;
    }
}

fn sync_rating_button(button: &gtk::Button, star: u8, rating: u8) {
    button.remove_css_class("rating-star-filled");
    button.remove_css_class("rating-star-empty");
    button.set_label(rating_star_label(star, rating));
    if star <= rating {
        button.add_css_class("rating-star-filled");
    } else {
        button.add_css_class("rating-star-empty");
    }
}

fn rating_star_label(star: u8, rating: u8) -> &'static str {
    if star <= rating {
        FILLED_STAR
    } else {
        EMPTY_STAR
    }
}

fn rating_after_click(current_rating: u8, clicked_star: u8) -> u8 {
    let clicked_star = clicked_star.min(MAX_RATING);
    if current_rating == clicked_star {
        0
    } else {
        clicked_star
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_a_different_star_sets_that_rating() {
        assert_eq!(rating_after_click(2, 4), 4);
    }

    #[test]
    fn clicking_the_current_rating_clears_rating_to_zero() {
        assert_eq!(rating_after_click(4, 4), 0);
    }

    #[test]
    fn rating_clicks_are_clamped_to_five_stars() {
        assert_eq!(rating_after_click(0, 9), 5);
    }
}
