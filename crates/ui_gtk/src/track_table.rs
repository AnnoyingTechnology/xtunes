// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::glib::variant::ToVariant;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering as CmpOrdering;
use std::collections::HashSet;
use std::rc::Rc;
use sustain_app_runtime::{Rating, TrackColumnEntry, TrackColumnLayout, TrackId};

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
pub(crate) type LayoutChangedCallback = Rc<dyn Fn(TrackColumnLayout)>;

/// A track column that participates in the persisted layout. Status and
/// filler columns are intentionally structural — they never appear in a
/// [`TrackColumnLayout`] and never move.
#[derive(Clone)]
struct ManagedColumn {
    column_id: &'static str,
    column: gtk::ColumnViewColumn,
}

#[derive(Clone)]
pub(crate) struct TrackTable {
    scroller: gtk::ScrolledWindow,
    table: gtk::ColumnView,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    status_bindings: StatusBindings,
    managed_columns: Rc<Vec<ManagedColumn>>,
    applying_layout: Rc<Cell<bool>>,
    layout_changed: Rc<RefCell<Option<LayoutChangedCallback>>>,
    pending_save: Rc<RefCell<Option<glib::SourceId>>>,
}

/// Debounce window for coalescing column-layout changes into a single save.
///
/// `notify::fixed-width` fires repeatedly while the user drags a column
/// boundary, so we must NOT write to SQLite on every property tick. 250 ms
/// is long enough to swallow a continuous drag (motion events keep the timer
/// resetting) yet short enough that a single visibility toggle feels
/// instantaneous and a pending save survives realistic close-window races
/// when [`TrackTable::flush_pending_layout_save`] is invoked on shutdown.
const LAYOUT_SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

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

    /// Updates the cached [`TrackTableRow`] for `track_id` in place,
    /// without emitting a `gio::ListModel::items-changed` signal.
    ///
    /// Used by single-track mutations whose cell widgets already update
    /// themselves on click (the rating stars are the canonical case —
    /// see `sync_rating_buttons` in the cells module). The visual
    /// update has already happened on the rendered widget; this call
    /// just keeps the row data the cell factory will re-bind to (when
    /// the user scrolls away and back, or when GTK re-binds for any
    /// other reason) in sync with the new state.
    ///
    /// Crucially, this does **not** splice or otherwise restructure the
    /// store. A splice would trigger `items-changed`, which the
    /// `ColumnView` treats as a structural event — focus is dropped
    /// and the scroll position resets to the top of the list. For a
    /// one-field change initiated by a click in the row itself, that
    /// is unacceptable UX.
    ///
    /// Trade-off: if the current sort is by the field that changed
    /// (e.g. Rating column sorted, then user re-rates the row), the
    /// row stays in its now-incorrect sorted position until the next
    /// full reflow. We accept that — losing the user's scroll/focus
    /// would be worse.
    ///
    /// Returns `true` when a matching row was found and updated.
    pub(crate) fn update_row(&self, track_id: TrackId, new_row: TrackTableRow) -> bool {
        let n_items = self.store.n_items();
        for position in 0..n_items {
            let Some(row_object) = self
                .store
                .item(position)
                .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
            else {
                continue;
            };
            let matches = row_object
                .try_borrow::<TrackTableRow>()
                .map(|row| row.track_id == Some(track_id))
                .unwrap_or(false);
            if !matches {
                continue;
            }
            // `BoxedAnyObject::borrow_mut` takes `&self`; the local
            // `row_object` is shadowed-immutable, but the inner
            // `RefCell` borrow is what we actually need.
            let mut row = row_object.borrow_mut::<TrackTableRow>();
            *row = new_row;
            return true;
        }
        false
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
            let is_target = row.track_id == Some(track_id);
            drop(row);

            if !is_target {
                continue;
            }
            self.table.scroll_to(
                position,
                None,
                gtk::ListScrollFlags::SELECT | gtk::ListScrollFlags::FOCUS,
                Some(vertical_scroll_info()),
            );
            return true;
        }
        false
    }

    /// Apply a persisted layout: reorder columns, set visibility, set widths.
    /// Any managed column missing from `layout` keeps its factory defaults and
    /// is appended after the explicit entries.
    ///
    /// The [`applying_layout`] guard is set for the duration so the resulting
    /// `notify::*` and `items-changed` signals do not loop back into a save.
    pub(crate) fn apply_layout(&self, layout: &TrackColumnLayout) {
        let _guard = ApplyLayoutGuard::enter(self.applying_layout.clone());
        let mut applied: HashSet<&'static str> = HashSet::new();
        // Position 0 is the status column; managed columns start at 1, and the
        // filler column is pushed to the end by the cascade of insert calls.
        let mut insert_at: u32 = 1;
        for entry in &layout.entries {
            if let Some(managed) = self
                .managed_columns
                .iter()
                .find(|managed| managed.column_id == entry.column_id.as_str())
            {
                managed.column.set_visible(entry.visible);
                managed
                    .column
                    .set_fixed_width(i32::try_from(entry.width_px).unwrap_or(i32::MAX));
                self.table.insert_column(insert_at, &managed.column);
                insert_at += 1;
                applied.insert(managed.column_id);
            }
        }
        for managed in self.managed_columns.iter() {
            if applied.contains(managed.column_id) {
                continue;
            }
            self.table.insert_column(insert_at, &managed.column);
            insert_at += 1;
        }
    }

    pub(crate) fn set_layout_changed_callback(&self, callback: LayoutChangedCallback) {
        *self.layout_changed.borrow_mut() = Some(callback);
    }

    /// Synchronously fires any pending debounced save. Call this from the
    /// window-close handler so a column tweak made within
    /// [`LAYOUT_SAVE_DEBOUNCE`] of shutdown is not lost.
    pub(crate) fn flush_pending_layout_save(&self) {
        let Some(source_id) = self.pending_save.borrow_mut().take() else {
            return;
        };
        source_id.remove();
        let Some(callback) = self.layout_changed.borrow().as_ref().cloned() else {
            return;
        };
        callback(read_current_layout(&self.table, &self.managed_columns));
    }
}

struct ApplyLayoutGuard {
    applying: Rc<Cell<bool>>,
}

impl ApplyLayoutGuard {
    fn enter(applying: Rc<Cell<bool>>) -> Self {
        applying.set(true);
        Self { applying }
    }
}

impl Drop for ApplyLayoutGuard {
    fn drop(&mut self) {
        self.applying.set(false);
    }
}

fn read_current_layout(
    table: &gtk::ColumnView,
    managed_columns: &[ManagedColumn],
) -> TrackColumnLayout {
    let columns_model = table.columns();
    let mut entries = Vec::with_capacity(managed_columns.len());
    for index in 0..columns_model.n_items() {
        let Some(item) = columns_model.item(index) else {
            continue;
        };
        let Ok(column) = item.downcast::<gtk::ColumnViewColumn>() else {
            continue;
        };
        let Some(managed) = managed_columns
            .iter()
            .find(|managed| managed.column.as_ptr() as *const () == column.as_ptr() as *const ())
        else {
            continue;
        };
        entries.push(TrackColumnEntry {
            column_id: managed.column_id.to_owned(),
            visible: managed.column.is_visible(),
            width_px: managed.column.fixed_width().max(0) as u32,
        });
    }
    TrackColumnLayout::new(entries)
}

fn vertical_scroll_info() -> gtk::ScrollInfo {
    let scroll_info = gtk::ScrollInfo::new();
    scroll_info.set_enable_horizontal(false);
    scroll_info.set_enable_vertical(true);
    scroll_info
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

    let context_menu =
        context_menu.map(|menu| TrackTableContextMenu::new(menu, selection.clone(), table.clone()));
    if let Some(context_menu) = &context_menu {
        context_menu.install_controller();
    }

    table.append_column(&build_status_column(
        playing_track_id.clone(),
        status_bindings.clone(),
        context_menu.clone(),
    ));

    let header_menu = build_column_visibility_menu();
    let column_actions = gio::SimpleActionGroup::new();
    let mut managed_columns: Vec<ManagedColumn> = Vec::with_capacity(TRACK_TABLE_COLUMNS.len());

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
        let column_for_action = table_column.clone();
        action.connect_activate(move |_action, _parameter| {
            let visible = !column_for_action.is_visible();
            column_for_action.set_visible(visible);
        });
        // Keep the menu checkmark in sync whenever the column's visibility
        // changes — whether the user toggled the action, dragged a separator,
        // or apply_layout() set it programmatically.
        let action_for_sync = action.clone();
        table_column.connect_notify_local(Some("visible"), move |column, _spec| {
            action_for_sync.set_state(&column.is_visible().to_variant());
        });
        column_actions.add_action(&action);
        table.append_column(&table_column);
        managed_columns.push(ManagedColumn {
            column_id: column.action_name(),
            column: table_column,
        });
    }
    table.append_column(&build_filler_column(context_menu.clone()));

    table.insert_action_group("columns", Some(&column_actions));

    let managed_columns: Rc<Vec<ManagedColumn>> = Rc::new(managed_columns);
    let applying_layout: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let layout_changed: Rc<RefCell<Option<LayoutChangedCallback>>> = Rc::new(RefCell::new(None));
    let pending_save: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

    install_layout_change_listeners(
        &table,
        managed_columns.clone(),
        applying_layout.clone(),
        layout_changed.clone(),
        pending_save.clone(),
    );

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
        table,
        store,
        selection,
        playing_track_id,
        status_bindings,
        managed_columns,
        applying_layout,
        layout_changed,
        pending_save,
    }
}

fn install_layout_change_listeners(
    table: &gtk::ColumnView,
    managed_columns: Rc<Vec<ManagedColumn>>,
    applying_layout: Rc<Cell<bool>>,
    layout_changed: Rc<RefCell<Option<LayoutChangedCallback>>>,
    pending_save: Rc<RefCell<Option<glib::SourceId>>>,
) {
    // Debounced scheduler. Each call cancels any prior pending save and queues
    // a new one LAYOUT_SAVE_DEBOUNCE in the future, so a continuous resize
    // drag (which fires notify::fixed-width per pixel) collapses to a single
    // SQLite write when the drag stops.
    let schedule: Rc<dyn Fn()> = {
        let table = table.clone();
        let managed_columns = managed_columns.clone();
        let applying_layout = applying_layout.clone();
        let layout_changed = layout_changed.clone();
        let pending_save = pending_save.clone();
        Rc::new(move || {
            if applying_layout.get() {
                return;
            }
            if let Some(previous) = pending_save.borrow_mut().take() {
                previous.remove();
            }
            let table = table.clone();
            let managed_columns = managed_columns.clone();
            let layout_changed = layout_changed.clone();
            let pending_save_clear = pending_save.clone();
            let source_id = glib::timeout_add_local_once(LAYOUT_SAVE_DEBOUNCE, move || {
                // The timer has now fired; release our handle to it before
                // doing work so flush_pending_layout_save() can run cleanly
                // even if the callback ends up triggering more changes.
                pending_save_clear.borrow_mut().take();
                let Some(callback) = layout_changed.borrow().as_ref().cloned() else {
                    return;
                };
                callback(read_current_layout(&table, &managed_columns));
            });
            *pending_save.borrow_mut() = Some(source_id);
        })
    };

    for managed in managed_columns.iter() {
        let schedule_for_width = schedule.clone();
        managed
            .column
            .connect_notify_local(Some("fixed-width"), move |_column, _spec| {
                schedule_for_width();
            });
        let schedule_for_visible = schedule.clone();
        managed
            .column
            .connect_notify_local(Some("visible"), move |_column, _spec| {
                schedule_for_visible();
            });
    }

    let schedule_for_reorder = schedule;
    table
        .columns()
        .connect_items_changed(move |_model, _position, _removed, _added| {
            schedule_for_reorder();
        });
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
