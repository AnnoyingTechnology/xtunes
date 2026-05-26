// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;
use gtk::{gdk, glib};

use sustain_app_runtime::{PlaylistItem, SmartPlaylistId, TrackId};

use super::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, SharedRuntime,
    sidebar_context::SidebarContextMenu,
};

mod drag_drop;
mod model;
mod row_context;

pub(crate) use drag_drop::{DropPosition, parse_tracks_payload, tracks_drag_payload};
use drag_drop::{SharedDropIndicator, attach_drag_and_drop};
use model::{SidebarItem, build_tree_model, select_item, selected_item};
use row_context::{
    SidebarRowContext, attach_rename_entry_signals, attach_row_context_menu, begin_rename,
};

pub(crate) type SidebarSelectionChangedCallback = Rc<dyn Fn(Option<SidebarSelection>)>;
pub(crate) type SidebarMoveCallback = Rc<dyn Fn(PlaylistItem, PlaylistItem, DropPosition)>;
pub(crate) type SidebarRenameCallback = Rc<dyn Fn(PlaylistItem, String)>;
pub(crate) type SidebarDeleteCallback = Rc<dyn Fn(PlaylistItem)>;
pub(crate) type SidebarTracksDropCallback = Rc<dyn Fn(PlaylistItem, Vec<TrackId>)>;
pub(crate) type SidebarEditSmartPlaylistCallback = Rc<dyn Fn(SmartPlaylistId)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SidebarSelection {
    Library,
    Item(PlaylistItem),
}

type MoveCallbackHolder = Rc<RefCell<Option<SidebarMoveCallback>>>;
type RenameCallbackHolder = Rc<RefCell<Option<SidebarRenameCallback>>>;
type DeleteCallbackHolder = Rc<RefCell<Option<SidebarDeleteCallback>>>;
type TracksDropCallbackHolder = Rc<RefCell<Option<SidebarTracksDropCallback>>>;
type EditSmartPlaylistCallbackHolder = Rc<RefCell<Option<SidebarEditSmartPlaylistCallback>>>;

#[derive(Clone)]
pub(crate) struct PlaylistSidebar {
    root: gtk::Box,
    selection: gtk::SingleSelection,
    library_row: gtk::Box,
    library_selected: Rc<Cell<bool>>,
    runtime: SharedRuntime,
    on_selection_changed: Rc<RefCell<Option<SidebarSelectionChangedCallback>>>,
    on_move: MoveCallbackHolder,
    on_rename: RenameCallbackHolder,
    on_delete: DeleteCallbackHolder,
    on_tracks_drop: TracksDropCallbackHolder,
    on_edit_smart_playlist: EditSmartPlaylistCallbackHolder,
    pending_rename: Rc<RefCell<Option<PlaylistItem>>>,
}

impl PlaylistSidebar {
    pub(crate) fn new(runtime: SharedRuntime) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("playlist-sidebar");
        root.set_vexpand(true);
        root.set_size_request(SIDEBAR_MIN_WIDTH, -1);

        let library_row = build_library_row();
        root.append(&library_row);

        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let title = gtk::Label::new(Some("Playlists"));
        title.set_margin_top(8);
        title.set_margin_end(8);
        title.set_margin_bottom(4);
        title.set_margin_start(8);
        title.set_xalign(0.0);
        root.append(&title);

        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let tree_model = build_tree_model(&runtime.borrow());
        let selection = gtk::SingleSelection::new(Some(tree_model));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let on_move: MoveCallbackHolder = Rc::new(RefCell::new(None));
        let on_rename: RenameCallbackHolder = Rc::new(RefCell::new(None));
        let on_delete: DeleteCallbackHolder = Rc::new(RefCell::new(None));
        let on_tracks_drop: TracksDropCallbackHolder = Rc::new(RefCell::new(None));
        let on_edit_smart_playlist: EditSmartPlaylistCallbackHolder = Rc::new(RefCell::new(None));
        let pending_rename: Rc<RefCell<Option<PlaylistItem>>> = Rc::new(RefCell::new(None));
        let list_view = gtk::ListView::new(
            Some(selection.clone()),
            Some(build_row_factory(
                on_move.clone(),
                on_rename.clone(),
                on_delete.clone(),
                on_tracks_drop.clone(),
                on_edit_smart_playlist.clone(),
                pending_rename.clone(),
            )),
        );
        list_view.add_css_class("playlist-sidebar-list");
        list_view.set_vexpand(true);
        list_view.set_single_click_activate(false);

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        scroller.set_hexpand(true);
        scroller.set_child(Some(&list_view));
        root.append(&scroller);

        let library_selected = Rc::new(Cell::new(false));
        let on_selection_changed: Rc<RefCell<Option<SidebarSelectionChangedCallback>>> =
            Rc::new(RefCell::new(None));

        connect_library_row(
            &library_row,
            &library_selected,
            &selection,
            on_selection_changed.clone(),
        );
        connect_selection_signal(
            &selection,
            &library_row,
            &library_selected,
            on_selection_changed.clone(),
        );

        // Library starts selected by default, matching the user's "Songs" mental
        // model — opening the Playlists view shows the whole library until a
        // specific playlist is picked.
        library_row.add_css_class("selected");
        library_selected.set(true);

        Self {
            root,
            selection,
            library_row,
            library_selected,
            runtime,
            on_selection_changed,
            on_move,
            on_rename,
            on_delete,
            on_tracks_drop,
            on_edit_smart_playlist,
            pending_rename,
        }
    }

    pub(crate) fn widget(&self) -> gtk::Box {
        self.root.clone()
    }

    pub(crate) fn set_selection_changed(&self, callback: SidebarSelectionChangedCallback) {
        let initial = callback.clone();
        self.on_selection_changed.replace(Some(callback));
        initial(self.current_selection());
    }

    pub(crate) fn set_move_callback(&self, callback: SidebarMoveCallback) {
        self.on_move.replace(Some(callback));
    }

    pub(crate) fn set_rename_callback(&self, callback: SidebarRenameCallback) {
        self.on_rename.replace(Some(callback));
    }

    pub(crate) fn set_delete_callback(&self, callback: SidebarDeleteCallback) {
        self.on_delete.replace(Some(callback));
    }

    pub(crate) fn set_tracks_drop_callback(&self, callback: SidebarTracksDropCallback) {
        self.on_tracks_drop.replace(Some(callback));
    }

    pub(crate) fn set_edit_smart_playlist_callback(
        &self,
        callback: SidebarEditSmartPlaylistCallback,
    ) {
        self.on_edit_smart_playlist.replace(Some(callback));
    }

    /// Arm an inline rename for `item` on the next bind that matches it.
    /// Designed for the "create then immediately name" flow: callers set this
    /// before calling [`Self::refresh`], so the new row enters edit mode the moment
    /// it is bound.
    pub(crate) fn arm_pending_rename(&self, item: PlaylistItem) {
        *self.pending_rename.borrow_mut() = Some(item);
    }

    pub(crate) fn current_selection(&self) -> Option<SidebarSelection> {
        if self.library_selected.get() {
            return Some(SidebarSelection::Library);
        }
        selected_item(&self.selection).map(SidebarSelection::Item)
    }

    pub(crate) fn select_item(&self, item: PlaylistItem) {
        self.library_selected.set(false);
        self.library_row.remove_css_class("selected");
        if !select_item(&self.selection, item) {
            self.select_library();
        }
    }

    fn select_library(&self) {
        self.library_selected.set(true);
        self.library_row.add_css_class("selected");
        self.selection.set_selected(gtk::INVALID_LIST_POSITION);
        if let Some(callback) = self.on_selection_changed.borrow().as_ref() {
            callback(Some(SidebarSelection::Library));
        }
    }

    pub(crate) fn install_context_menu(&self, menu: SidebarContextMenu) {
        menu.install_on(self.root.upcast_ref::<gtk::Widget>());
    }

    pub(crate) fn refresh(&self) {
        let previous = self.current_selection();
        let tree_model = build_tree_model(&self.runtime.borrow());
        // Suppress the selection callback while we swap the model and
        // restore the previous selection. `set_model` and `select_item`
        // each emit `selected_notify` synchronously, and the connected
        // notify handler would otherwise fire the callback up to twice
        // — each call rebuilds the playlists table, which dominates
        // refresh cost on a large library. Suspend, do the surgery,
        // restore, and fire the callback exactly once with the final
        // selection.
        let suspended = self.on_selection_changed.borrow_mut().take();
        self.selection.set_model(Some(&tree_model));
        match previous {
            Some(SidebarSelection::Item(item)) => {
                self.library_selected.set(false);
                self.library_row.remove_css_class("selected");
                if !select_item(&self.selection, item) {
                    self.library_selected.set(true);
                    self.library_row.add_css_class("selected");
                }
            }
            Some(SidebarSelection::Library) | None => {
                // Library stays selected (its CSS class was unchanged).
                self.selection.set_selected(gtk::INVALID_LIST_POSITION);
            }
        }
        *self.on_selection_changed.borrow_mut() = suspended;
        if let Some(callback) = self.on_selection_changed.borrow().as_ref() {
            callback(self.current_selection());
        }
    }
}

fn build_library_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("playlist-sidebar-library");
    row.set_margin_top(6);
    row.set_margin_end(6);
    row.set_margin_start(6);
    row.set_margin_bottom(2);

    let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    icon.add_css_class("playlist-sidebar-icon");

    let label = gtk::Label::new(Some("Library"));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    row.append(&icon);
    row.append(&label);
    row
}

fn connect_library_row(
    library_row: &gtk::Box,
    library_selected: &Rc<Cell<bool>>,
    selection: &gtk::SingleSelection,
    on_selection_changed: Rc<RefCell<Option<SidebarSelectionChangedCallback>>>,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_PRIMARY);

    let library_row_for_click = library_row.clone();
    let library_selected_for_click = library_selected.clone();
    let selection_for_click = selection.clone();
    gesture.connect_pressed(move |gesture, _n_press, _x, _y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if !library_selected_for_click.get() {
            library_selected_for_click.set(true);
            library_row_for_click.add_css_class("selected");
            selection_for_click.set_selected(gtk::INVALID_LIST_POSITION);
        }
        if let Some(callback) = on_selection_changed.borrow().as_ref() {
            callback(Some(SidebarSelection::Library));
        }
    });
    library_row.add_controller(gesture);
}

fn connect_selection_signal(
    selection: &gtk::SingleSelection,
    library_row: &gtk::Box,
    library_selected: &Rc<Cell<bool>>,
    on_selection_changed: Rc<RefCell<Option<SidebarSelectionChangedCallback>>>,
) {
    let selection_clone = selection.clone();
    let library_row = library_row.clone();
    let library_selected = library_selected.clone();
    selection.connect_selected_notify(move |_selection| {
        let item = selected_item(&selection_clone);
        let new_selection = if let Some(item) = item {
            if library_selected.get() {
                library_selected.set(false);
                library_row.remove_css_class("selected");
            }
            Some(SidebarSelection::Item(item))
        } else if library_selected.get() {
            Some(SidebarSelection::Library)
        } else {
            None
        };
        if let Some(callback) = on_selection_changed.borrow().as_ref() {
            callback(new_selection);
        }
    });
}

fn build_row_factory(
    on_move: MoveCallbackHolder,
    on_rename: RenameCallbackHolder,
    on_delete: DeleteCallbackHolder,
    on_tracks_drop: TracksDropCallbackHolder,
    on_edit_smart_playlist: EditSmartPlaylistCallbackHolder,
    pending_rename: Rc<RefCell<Option<PlaylistItem>>>,
) -> gtk::SignalListItemFactory {
    let current_indicator: SharedDropIndicator = Rc::new(RefCell::new(None));
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_factory, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        // Inner Box no longer carries the visual frame class; the frame
        // (padding, border-radius, drop indicators) lives on the TreeExpander
        // below so it spans the full row width.

        let icon = gtk::Image::new();
        icon.add_css_class("playlist-sidebar-icon");

        let name_stack = gtk::Stack::new();
        name_stack.set_hexpand(true);
        name_stack.set_transition_type(gtk::StackTransitionType::None);
        // Without this, the Stack reserves the tall rename Entry's natural
        // height even when the compact Label is the visible child, padding
        // every sidebar row to ~32px. Sizing to the visible child keeps the
        // row tight in display mode and lets the entry briefly grow when
        // rename starts.
        name_stack.set_vhomogeneous(false);

        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_hexpand(true);

        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.add_css_class("playlist-sidebar-rename-entry");

        name_stack.add_named(&label, Some("label"));
        name_stack.add_named(&entry, Some("entry"));
        name_stack.set_visible_child_name("label");

        row.append(&icon);
        row.append(&name_stack);

        let expander = gtk::TreeExpander::new();
        expander.add_css_class("playlist-sidebar-row");
        expander.set_child(Some(&row));
        list_item.set_child(Some(&expander));
    });

    factory.connect_bind(move |_factory, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(expander_widget) = list_item.child() else {
            return;
        };
        let Ok(expander) = expander_widget.downcast::<gtk::TreeExpander>() else {
            return;
        };
        let Some(item_object) = list_item.item() else {
            return;
        };
        let Ok(tree_row) = item_object.downcast::<gtk::TreeListRow>() else {
            return;
        };
        expander.set_list_row(Some(&tree_row));

        let Some(inner_object) = tree_row.item() else {
            return;
        };
        let Some(boxed) = inner_object.downcast_ref::<glib::BoxedAnyObject>() else {
            return;
        };
        let Ok(sidebar_item) = boxed.try_borrow::<SidebarItem>() else {
            return;
        };
        let playlist_item = sidebar_item.item;
        let label_text = sidebar_item.name.clone();
        let icon_name = sidebar_item.icon_name();
        drop(sidebar_item);

        let Some(row_widget) = expander.child() else {
            return;
        };
        let Ok(row) = row_widget.downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon_widget) = row.first_child() else {
            return;
        };
        let Ok(icon) = icon_widget.downcast::<gtk::Image>() else {
            return;
        };
        let Some(stack_widget) = icon.next_sibling() else {
            return;
        };
        let Ok(name_stack) = stack_widget.downcast::<gtk::Stack>() else {
            return;
        };
        let Some(label) = name_stack
            .child_by_name("label")
            .and_then(|child| child.downcast::<gtk::Label>().ok())
        else {
            return;
        };
        let Some(entry) = name_stack
            .child_by_name("entry")
            .and_then(|child| child.downcast::<gtk::Entry>().ok())
        else {
            return;
        };

        icon.set_icon_name(Some(icon_name));
        label.set_text(&label_text);
        entry.set_text(&label_text);
        name_stack.set_visible_child_name("label");

        attach_drag_and_drop(
            expander.upcast_ref::<gtk::Widget>(),
            playlist_item,
            on_move.clone(),
            on_tracks_drop.clone(),
            current_indicator.clone(),
        );
        attach_row_context_menu(
            expander.upcast_ref::<gtk::Widget>(),
            SidebarRowContext {
                item: playlist_item,
                current_name: label_text.clone(),
                name_stack: name_stack.clone(),
                label: label.clone(),
                entry: entry.clone(),
                on_delete: on_delete.clone(),
                on_edit_smart_playlist: on_edit_smart_playlist.clone(),
            },
        );
        attach_rename_entry_signals(
            &entry,
            &name_stack,
            &label,
            playlist_item,
            on_rename.clone(),
        );

        // If a caller armed `arm_pending_rename` for this item, immediately
        // enter rename mode now that the row is bound and its widgets exist.
        let should_rename = {
            let mut pending = pending_rename.borrow_mut();
            if pending.as_ref() == Some(&playlist_item) {
                *pending = None;
                true
            } else {
                false
            }
        };
        if should_rename {
            begin_rename(&name_stack, &label, &entry);
        }
    });

    factory
}

pub(crate) fn build_content_area(sidebar: &gtk::Box, main_content: &gtk::Box) -> gtk::Paned {
    let content_area = gtk::Paned::new(gtk::Orientation::Horizontal);
    content_area.set_hexpand(true);
    content_area.set_vexpand(true);
    content_area.set_wide_handle(false);
    content_area.set_resize_start_child(false);
    content_area.set_shrink_start_child(false);
    content_area.set_resize_end_child(true);
    content_area.set_shrink_end_child(false);
    content_area.set_start_child(Some(sidebar));
    content_area.set_end_child(Some(main_content));
    content_area.set_position(SIDEBAR_DEFAULT_WIDTH);
    content_area.connect_position_notify(clamp_sidebar_width);
    content_area
}

fn clamp_sidebar_width(content_area: &gtk::Paned) {
    let current_width = content_area.position();
    let clamped_width = current_width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
    if clamped_width != current_width {
        content_area.set_position(clamped_width);
    }
}
