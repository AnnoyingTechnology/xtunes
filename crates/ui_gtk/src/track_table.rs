// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::glib::variant::ToVariant;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::cmp::Ordering as CmpOrdering;
use std::rc::Rc;
use sustain_app_runtime::{Rating, TrackId};

use super::track_context::TrackRowContextMenu;
use cells::{
    StatusBindings, TrackTableContextMenu, build_filler_column, build_rating_cell_factory,
    build_status_column, build_text_cell_factory,
};
use columns::{TRACK_TABLE_COLUMNS, TrackTableColumn};
pub(crate) use row::TrackTableRow;

mod cells;
mod columns;
mod row;

pub(crate) type TrackActivatedCallback = Rc<dyn Fn(TrackId)>;
pub(crate) type RatingChangedCallback = Rc<dyn Fn(TrackId, Rating) -> bool>;

#[derive(Clone)]
pub(crate) struct TrackTable {
    scroller: gtk::ScrolledWindow,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    status_bindings: StatusBindings,
}

impl TrackTable {
    pub(crate) fn widget(&self) -> gtk::ScrolledWindow {
        self.scroller.clone()
    }

    pub(crate) fn replace_rows(&self, rows: Vec<TrackTableRow>) {
        self.store.remove_all();
        for row in rows {
            self.store.append(&glib::BoxedAnyObject::new(row));
        }
    }

    pub(crate) fn set_playing_track_id(&self, playing_track_id: Option<TrackId>) {
        if self.playing_track_id.get() == playing_track_id {
            return;
        }
        self.playing_track_id.set(playing_track_id);
        self.status_bindings.refresh(playing_track_id);
    }

    /// Finds the row whose track matches `track_id` in the current sort order,
    /// selects it (clearing any prior selection), and scrolls it into the
    /// viewport. Returns `false` when no row matches — callers use that as the
    /// signal to fall back to a different view (Songs is the fallback for
    /// Ctrl-L when the playing track is not in the current view's contents).
    pub(crate) fn reveal_track(&self, track_id: TrackId) -> bool {
        let n_items = self.selection.n_items();
        for position in 0..n_items {
            let Some(row_object) = self
                .selection
                .item(position)
                .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
            else {
                continue;
            };
            let Ok(row) = row_object.try_borrow::<TrackTableRow>() else {
                continue;
            };
            if row.track_id != Some(track_id) {
                continue;
            }
            drop(row);
            self.selection.select_item(position, true);
            // The scroll math reads `vadj.upper()` and the selection's
            // `n_items` to derive row height. Right after a layout-triggering
            // change (e.g. the Playlists table was just refreshed by the
            // sidebar selection callback, or the user switched views) those
            // can be stale within the same frame, producing a no-op scroll on
            // the first Ctrl-L. Deferring to idle lets GTK finish the pending
            // layout before we read the adjustment, so the first press lands.
            let scroller = self.scroller.clone();
            let selection = self.selection.clone();
            glib::idle_add_local_once(move || {
                scroll_row_into_view(&scroller, position, selection.n_items());
            });
            return true;
        }
        false
    }
}

/// Centers row `position` in the viewport by setting the ScrolledWindow's
/// vertical adjustment directly. GTK 4.12 added `ColumnView::scroll_to`, but
/// the project targets GTK 4.10 (Debian-first); the adjustment math here
/// derives row height from the live `upper / n_items` instead of hardcoding a
/// pixel constant, so the result stays correct as the row CSS evolves.
///
/// KNOWN FLAKY: the Playlists view occasionally needs a second Ctrl-L for
/// the scroll to take effect. The selection always lands on the first
/// press, but `vadj.upper` can still be stale on the idle tick after a
/// recent layout change (sidebar selection → `replace_rows` → ColumnView
/// relayout). When the GTK 4.18 bump lands, replace this entire helper
/// with `ColumnView::scroll_to(position, ListScrollFlags::SELECT)` and
/// drop the manual idle defer in `TrackTable::reveal_track`.
fn scroll_row_into_view(scroller: &gtk::ScrolledWindow, position: u32, n_items: u32) {
    if n_items == 0 {
        return;
    }
    let vadj = scroller.vadjustment();
    let upper = vadj.upper();
    let page = vadj.page_size();
    if upper <= 0.0 || page <= 0.0 {
        return;
    }
    let row_height = upper / n_items as f64;
    let target_y = position as f64 * row_height;
    let max_value = (upper - page).max(0.0);
    let centered = (target_y - (page - row_height) / 2.0)
        .max(0.0)
        .min(max_value);
    vadj.set_value(centered);
}

pub(crate) fn build_track_table(
    rows: Vec<TrackTableRow>,
    track_activated: Option<TrackActivatedCallback>,
    context_menu: Option<TrackRowContextMenu>,
    rating_changed: Option<RatingChangedCallback>,
) -> TrackTable {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for row in rows {
        store.append(&glib::BoxedAnyObject::new(row));
    }

    let table = gtk::ColumnView::new(None::<gtk::SelectionModel>);
    table.add_css_class("track-table");
    table.set_hexpand(true);
    table.set_vexpand(true);
    table.set_reorderable(true);
    table.set_show_column_separators(false);
    table.set_show_row_separators(false);
    table.set_single_click_activate(false);

    let playing_track_id: Rc<Cell<Option<TrackId>>> = Rc::new(Cell::new(None));
    let status_bindings = StatusBindings::default();

    let sorted_rows = gtk::SortListModel::new(Some(store.clone()), table.sorter());
    let selection = gtk::MultiSelection::new(Some(sorted_rows));

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_hexpand(true);

    let context_menu = context_menu
        .map(|menu| TrackTableContextMenu::new(menu, selection.clone(), scroller.clone()));

    table.append_column(&build_status_column(
        playing_track_id.clone(),
        status_bindings.clone(),
        context_menu.clone(),
    ));

    let header_menu = build_column_visibility_menu();
    let column_actions = gio::SimpleActionGroup::new();

    for column in TRACK_TABLE_COLUMNS.iter().copied() {
        let table_column = build_table_column(
            column,
            &header_menu,
            context_menu.clone(),
            rating_changed.clone(),
        );
        let action = gio::SimpleAction::new_stateful(
            column.action_name(),
            None,
            &column.default_visible().to_variant(),
        );
        let table_column_for_action = table_column.clone();
        action.connect_activate(move |action, _parameter| {
            let visible = !table_column_for_action.is_visible();
            table_column_for_action.set_visible(visible);
            action.set_state(&visible.to_variant());
        });
        column_actions.add_action(&action);
        table.append_column(&table_column);
    }
    table.append_column(&build_filler_column(context_menu.clone()));

    table.insert_action_group("columns", Some(&column_actions));

    if let Some(track_activated) = track_activated {
        let selection_for_activate = selection.clone();
        table.connect_activate(move |_table, position| {
            let Some(track_id) = selection_for_activate
                .item(position)
                .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
                .and_then(|row_object| {
                    row_object
                        .try_borrow::<TrackTableRow>()
                        .ok()
                        .and_then(|row| row.track_id)
                })
            else {
                return;
            };

            track_activated(track_id);
        });
    }
    table.set_model(Some(&selection));

    scroller.set_child(Some(&table));
    TrackTable {
        scroller,
        store,
        selection,
        playing_track_id,
        status_bindings,
    }
}

fn build_table_column(
    column: TrackTableColumn,
    header_menu: &gio::Menu,
    context_menu: Option<TrackTableContextMenu>,
    rating_changed: Option<RatingChangedCallback>,
) -> gtk::ColumnViewColumn {
    let factory = if column == TrackTableColumn::Rating {
        build_rating_cell_factory(context_menu, rating_changed)
    } else {
        build_text_cell_factory(column, context_menu)
    };
    let table_column = gtk::ColumnViewColumn::new(Some(column.title()), Some(factory));
    table_column.set_resizable(true);
    table_column.set_expand(column.expands());
    table_column.set_fixed_width(column.default_width());
    table_column.set_visible(column.default_visible());
    table_column.set_header_menu(Some(header_menu));

    let sorter =
        gtk::CustomSorter::new(move |left, right| compare_track_objects(column, left, right));
    table_column.set_sorter(Some(&sorter));

    table_column
}

fn build_column_visibility_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let columns = gio::Menu::new();
    for column in TRACK_TABLE_COLUMNS {
        columns.append(
            Some(column.title()),
            Some(&format!("columns.{}", column.action_name())),
        );
    }
    menu.append_section(Some("Columns"), &columns);
    menu
}

fn compare_track_objects(
    column: TrackTableColumn,
    left: &glib::Object,
    right: &glib::Object,
) -> gtk::Ordering {
    let Some(left) = left.downcast_ref::<glib::BoxedAnyObject>() else {
        return gtk::Ordering::Equal;
    };
    let Some(right) = right.downcast_ref::<glib::BoxedAnyObject>() else {
        return gtk::Ordering::Equal;
    };
    let Ok(left) = left.try_borrow::<TrackTableRow>() else {
        return gtk::Ordering::Equal;
    };
    let Ok(right) = right.try_borrow::<TrackTableRow>() else {
        return gtk::Ordering::Equal;
    };

    to_gtk_ordering(column.compare(&left, &right))
}

fn to_gtk_ordering(ordering: CmpOrdering) -> gtk::Ordering {
    match ordering {
        CmpOrdering::Less => gtk::Ordering::Smaller,
        CmpOrdering::Equal => gtk::Ordering::Equal,
        CmpOrdering::Greater => gtk::Ordering::Larger,
    }
}
