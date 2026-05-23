// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::glib::variant::ToVariant;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::Cell;
use std::cmp::Ordering as CmpOrdering;
use std::rc::Rc;
use xtunes_app_runtime::{Rating, TrackId};

use super::sidebar::tracks_drag_payload;
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

    attach_tracks_drag_source(&table, &selection);

    scroller.set_child(Some(&table));
    TrackTable {
        scroller,
        store,
        playing_track_id,
        status_bindings,
    }
}

fn attach_tracks_drag_source(table: &gtk::ColumnView, selection: &gtk::MultiSelection) {
    let drag_source = gtk::DragSource::new();
    drag_source.set_actions(gdk::DragAction::COPY);

    let selection = selection.clone();
    drag_source.connect_prepare(move |_source, _x, _y| {
        let track_ids = collect_selected_track_ids(&selection);
        if track_ids.is_empty() {
            return None;
        }
        Some(gdk::ContentProvider::for_value(
            &tracks_drag_payload(&track_ids).to_value(),
        ))
    });
    table.add_controller(drag_source);
}

fn collect_selected_track_ids(selection: &gtk::MultiSelection) -> Vec<TrackId> {
    let bitset = selection.selection();
    let n = selection.n_items();
    let mut track_ids = Vec::new();
    for index in 0..n {
        if !bitset.contains(index) {
            continue;
        }
        let Some(item) = selection.item(index) else {
            continue;
        };
        let Ok(boxed) = item.downcast::<glib::BoxedAnyObject>() else {
            continue;
        };
        let Ok(row) = boxed.try_borrow::<TrackTableRow>() else {
            continue;
        };
        if let Some(track_id) = row.track_id {
            track_ids.push(track_id);
        }
    }
    track_ids
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
