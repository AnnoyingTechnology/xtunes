// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use xtunes_app_runtime::{
    ApplicationRuntime, Playlist, PlaylistFolder, PlaylistFolderId, PlaylistId, PlaylistItem,
    SmartPlaylist, SmartPlaylistId, TrackId,
};

use super::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, SharedRuntime,
    sidebar_context::SidebarContextMenu,
};

pub(crate) type SidebarSelectionChangedCallback = Rc<dyn Fn(Option<SidebarSelection>)>;
pub(crate) type SidebarMoveCallback = Rc<dyn Fn(PlaylistItem, PlaylistItem, DropPosition)>;
pub(crate) type SidebarRenameCallback = Rc<dyn Fn(PlaylistItem, String)>;
pub(crate) type SidebarDeleteCallback = Rc<dyn Fn(PlaylistItem)>;
pub(crate) type SidebarTracksDropCallback = Rc<dyn Fn(PlaylistItem, Vec<TrackId>)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SidebarSelection {
    Library,
    Item(PlaylistItem),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DropPosition {
    Above,
    Below,
    Into,
}

pub(crate) fn drop_position_from_motion(
    y: f64,
    row_height: f64,
    target_is_folder: bool,
) -> DropPosition {
    if row_height <= 0.0 {
        return if target_is_folder {
            DropPosition::Into
        } else {
            DropPosition::Above
        };
    }
    let ratio = (y / row_height).clamp(0.0, 1.0);
    if target_is_folder {
        if ratio < 0.25 {
            DropPosition::Above
        } else if ratio > 0.75 {
            DropPosition::Below
        } else {
            DropPosition::Into
        }
    } else if ratio < 0.5 {
        DropPosition::Above
    } else {
        DropPosition::Below
    }
}

#[derive(Clone, Debug)]
struct SidebarItem {
    name: String,
    item: PlaylistItem,
}

impl SidebarItem {
    fn icon_name(&self) -> &'static str {
        match self.item {
            PlaylistItem::Folder(_) => "folder-symbolic",
            PlaylistItem::Playlist(_) => "view-list-symbolic",
            PlaylistItem::SmartPlaylist(_) => "emblem-system-symbolic",
        }
    }
}

#[derive(Default)]
struct SidebarSnapshot {
    items_by_parent: BTreeMap<Option<PlaylistFolderId>, Vec<SidebarItem>>,
}

impl SidebarSnapshot {
    fn from_runtime(runtime: &ApplicationRuntime) -> Self {
        Self::build(
            runtime.playlist_folders(),
            runtime.playlists(),
            runtime.smart_playlists(),
        )
    }

    fn build(
        folders: &[PlaylistFolder],
        playlists: &[Playlist],
        smart_playlists: &[SmartPlaylist],
    ) -> Self {
        let mut items_by_parent: BTreeMap<Option<PlaylistFolderId>, Vec<(u32, SidebarItem)>> =
            BTreeMap::new();

        for folder in folders {
            items_by_parent
                .entry(folder.parent_folder_id)
                .or_default()
                .push((
                    folder.position,
                    SidebarItem {
                        name: folder.name.clone(),
                        item: PlaylistItem::Folder(folder.id),
                    },
                ));
        }
        for playlist in playlists {
            items_by_parent
                .entry(playlist.parent_folder_id)
                .or_default()
                .push((
                    playlist.position,
                    SidebarItem {
                        name: playlist.name.clone(),
                        item: PlaylistItem::Playlist(playlist.id),
                    },
                ));
        }
        for smart in smart_playlists {
            items_by_parent
                .entry(smart.parent_folder_id)
                .or_default()
                .push((
                    smart.position,
                    SidebarItem {
                        name: smart.name.clone(),
                        item: PlaylistItem::SmartPlaylist(smart.id),
                    },
                ));
        }

        let items_by_parent = items_by_parent
            .into_iter()
            .map(|(parent, mut bucket)| {
                bucket.sort_by_key(|(position, _)| *position);
                (parent, bucket.into_iter().map(|(_, item)| item).collect())
            })
            .collect();

        Self { items_by_parent }
    }

    fn items_under(&self, parent: Option<PlaylistFolderId>) -> &[SidebarItem] {
        self.items_by_parent
            .get(&parent)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

type MoveCallbackHolder = Rc<RefCell<Option<SidebarMoveCallback>>>;
type RenameCallbackHolder = Rc<RefCell<Option<SidebarRenameCallback>>>;
type DeleteCallbackHolder = Rc<RefCell<Option<SidebarDeleteCallback>>>;
type TracksDropCallbackHolder = Rc<RefCell<Option<SidebarTracksDropCallback>>>;

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
        let list_view = gtk::ListView::new(
            Some(selection.clone()),
            Some(build_row_factory(
                on_move.clone(),
                on_rename.clone(),
                on_delete.clone(),
                on_tracks_drop.clone(),
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

    pub(crate) fn current_selection(&self) -> Option<SidebarSelection> {
        if self.library_selected.get() {
            return Some(SidebarSelection::Library);
        }
        selected_item(&self.selection).map(SidebarSelection::Item)
    }

    pub(crate) fn install_context_menu(&self, menu: SidebarContextMenu) {
        menu.install_on(self.root.upcast_ref::<gtk::Widget>());
    }

    pub(crate) fn refresh(&self) {
        let previous = self.current_selection();
        let tree_model = build_tree_model(&self.runtime.borrow());
        self.selection.set_model(Some(&tree_model));
        match previous {
            Some(SidebarSelection::Item(item)) => {
                self.library_selected.set(false);
                self.library_row.remove_css_class("selected");
                select_item(&self.selection, item);
            }
            Some(SidebarSelection::Library) | None => {
                // Library stays selected (its CSS class was unchanged).
                self.selection.set_selected(gtk::INVALID_LIST_POSITION);
            }
        }
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

fn selected_item(selection: &gtk::SingleSelection) -> Option<PlaylistItem> {
    let object = selection.selected_item()?;
    let tree_row = object.downcast_ref::<gtk::TreeListRow>()?;
    let inner = tree_row.item()?;
    let boxed = inner.downcast_ref::<glib::BoxedAnyObject>()?;
    let sidebar_item = boxed.try_borrow::<SidebarItem>().ok()?;
    Some(sidebar_item.item)
}

fn select_item(selection: &gtk::SingleSelection, target: PlaylistItem) {
    let n = selection.n_items();
    for index in 0..n {
        let Some(object) = selection.item(index) else {
            continue;
        };
        let Some(tree_row) = object.downcast_ref::<gtk::TreeListRow>() else {
            continue;
        };
        let Some(inner) = tree_row.item() else {
            continue;
        };
        let Some(boxed) = inner.downcast_ref::<glib::BoxedAnyObject>() else {
            continue;
        };
        let Ok(sidebar_item) = boxed.try_borrow::<SidebarItem>() else {
            continue;
        };
        if sidebar_item.item == target {
            selection.set_selected(index);
            return;
        }
    }
    selection.set_selected(gtk::INVALID_LIST_POSITION);
}

fn build_tree_model(runtime: &ApplicationRuntime) -> gtk::TreeListModel {
    let snapshot = Rc::new(SidebarSnapshot::from_runtime(runtime));
    let root_store = list_store_for(snapshot.items_under(None));

    let snapshot_for_children = snapshot.clone();
    gtk::TreeListModel::new(
        root_store,
        false,
        true,
        move |object| children_for(object, &snapshot_for_children),
    )
}

fn children_for(object: &glib::Object, snapshot: &SidebarSnapshot) -> Option<gio::ListModel> {
    let boxed = object.downcast_ref::<glib::BoxedAnyObject>()?;
    let sidebar_item = boxed.try_borrow::<SidebarItem>().ok()?;
    let PlaylistItem::Folder(folder_id) = sidebar_item.item else {
        return None;
    };
    Some(list_store_for(snapshot.items_under(Some(folder_id))).upcast())
}

fn list_store_for(items: &[SidebarItem]) -> gio::ListStore {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for item in items {
        store.append(&glib::BoxedAnyObject::new(item.clone()));
    }
    store
}

fn build_row_factory(
    on_move: MoveCallbackHolder,
    on_rename: RenameCallbackHolder,
    on_delete: DeleteCallbackHolder,
    on_tracks_drop: TracksDropCallbackHolder,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_factory, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_margin_top(2);
        row.set_margin_bottom(2);

        let icon = gtk::Image::new();
        icon.add_css_class("playlist-sidebar-icon");

        let name_stack = gtk::Stack::new();
        name_stack.set_hexpand(true);
        name_stack.set_transition_type(gtk::StackTransitionType::None);

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
            &row,
            playlist_item,
            on_move.clone(),
            on_tracks_drop.clone(),
        );
        attach_row_context_menu(
            &row,
            playlist_item,
            label_text.clone(),
            name_stack.clone(),
            label.clone(),
            entry.clone(),
            on_delete.clone(),
        );
        attach_rename_entry_signals(
            &entry,
            &name_stack,
            &label,
            playlist_item,
            on_rename.clone(),
        );
    });

    factory
}

fn attach_row_context_menu(
    row: &gtk::Box,
    item: PlaylistItem,
    current_name: String,
    name_stack: gtk::Stack,
    label: gtk::Label,
    entry: gtk::Entry,
    on_delete: DeleteCallbackHolder,
) {
    remove_secondary_gestures(row);

    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    let row_widget = row.clone();
    gesture.connect_pressed(move |gesture, _n_press, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        popup_row_context_menu(
            &row_widget,
            item,
            current_name.clone(),
            name_stack.clone(),
            label.clone(),
            entry.clone(),
            on_delete.clone(),
            x,
            y,
        );
    });
    row.add_controller(gesture);
}

fn remove_secondary_gestures(widget: &gtk::Box) {
    let controllers = widget.observe_controllers();
    let mut to_remove: Vec<gtk::EventController> = Vec::new();
    for index in 0..controllers.n_items() {
        let Some(object) = controllers.item(index) else {
            continue;
        };
        let Some(gesture) = object.downcast_ref::<gtk::GestureClick>() else {
            continue;
        };
        if gesture.button() != gdk::BUTTON_SECONDARY {
            continue;
        }
        if let Ok(controller) = object.downcast::<gtk::EventController>() {
            to_remove.push(controller);
        }
    }
    for controller in to_remove {
        widget.remove_controller(&controller);
    }
}

#[allow(clippy::too_many_arguments)]
fn popup_row_context_menu(
    anchor: &gtk::Box,
    item: PlaylistItem,
    current_name: String,
    name_stack: gtk::Stack,
    label: gtk::Label,
    entry: gtk::Entry,
    on_delete: DeleteCallbackHolder,
    x: f64,
    y: f64,
) {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_parent(anchor);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("sidebar-context-menu");

    let rename_button = row_action_button("Rename");
    let popover_for_rename = popover.clone();
    let name_stack_for_rename = name_stack.clone();
    let label_for_rename = label.clone();
    let entry_for_rename = entry.clone();
    rename_button.connect_clicked(move |_| {
        popover_for_rename.popdown();
        begin_rename(&name_stack_for_rename, &label_for_rename, &entry_for_rename);
    });
    content.append(&rename_button);

    let delete_button = row_action_button(delete_label_for(item));
    delete_button.add_css_class("destructive-action");
    let popover_for_delete = popover.clone();
    let anchor_for_delete = anchor.clone();
    delete_button.connect_clicked(move |_| {
        popover_for_delete.popdown();
        confirm_and_delete(
            anchor_for_delete.upcast_ref::<gtk::Widget>(),
            item,
            current_name.clone(),
            on_delete.clone(),
        );
    });
    content.append(&delete_button);

    popover.set_child(Some(&content));

    let popover_for_close = popover.clone();
    popover.connect_closed(move |_| {
        popover_for_close.unparent();
    });

    let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
    popover.set_pointing_to(Some(&rect));
    popover.popup();
}

fn row_action_button(label_text: &str) -> gtk::Button {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Fill);
    label.set_hexpand(true);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("sidebar-context-menu-item");
    button.set_child(Some(&label));
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button
}

fn delete_label_for(item: PlaylistItem) -> &'static str {
    match item {
        PlaylistItem::Folder(_) => "Delete Folder…",
        PlaylistItem::Playlist(_) => "Delete Playlist",
        PlaylistItem::SmartPlaylist(_) => "Delete Smart Playlist",
    }
}

fn begin_rename(name_stack: &gtk::Stack, label: &gtk::Label, entry: &gtk::Entry) {
    entry.set_text(&label.text());
    name_stack.set_visible_child_name("entry");
    entry.grab_focus();
    entry.select_region(0, -1);
}

fn cancel_rename(name_stack: &gtk::Stack, label: &gtk::Label, entry: &gtk::Entry) {
    entry.set_text(&label.text());
    name_stack.set_visible_child_name("label");
}

fn commit_rename(
    name_stack: &gtk::Stack,
    label: &gtk::Label,
    entry: &gtk::Entry,
    item: PlaylistItem,
    on_rename: &RenameCallbackHolder,
) {
    let new_name = entry.text().to_string();
    let trimmed = new_name.trim();
    if trimmed.is_empty() || trimmed == label.text().as_str() {
        cancel_rename(name_stack, label, entry);
        return;
    }
    name_stack.set_visible_child_name("label");
    if let Some(callback) = on_rename.borrow().as_ref() {
        callback(item, trimmed.to_owned());
    }
}

fn attach_rename_entry_signals(
    entry: &gtk::Entry,
    name_stack: &gtk::Stack,
    label: &gtk::Label,
    item: PlaylistItem,
    on_rename: RenameCallbackHolder,
) {
    remove_focus_controllers(entry);

    let name_stack_for_activate = name_stack.clone();
    let label_for_activate = label.clone();
    let on_rename_for_activate = on_rename.clone();
    entry.connect_activate(move |entry| {
        commit_rename(
            &name_stack_for_activate,
            &label_for_activate,
            entry,
            item,
            &on_rename_for_activate,
        );
    });

    let key_controller = gtk::EventControllerKey::new();
    let name_stack_for_escape = name_stack.clone();
    let label_for_escape = label.clone();
    let entry_for_escape = entry.clone();
    key_controller.connect_key_pressed(move |_controller, key, _keycode, _state| {
        if key == gdk::Key::Escape {
            cancel_rename(&name_stack_for_escape, &label_for_escape, &entry_for_escape);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(key_controller);

    let focus_controller = gtk::EventControllerFocus::new();
    let name_stack_for_focus = name_stack.clone();
    let label_for_focus = label.clone();
    let entry_for_focus = entry.clone();
    let on_rename_for_focus = on_rename.clone();
    focus_controller.connect_leave(move |_controller| {
        if name_stack_for_focus.visible_child_name().as_deref() == Some("entry") {
            commit_rename(
                &name_stack_for_focus,
                &label_for_focus,
                &entry_for_focus,
                item,
                &on_rename_for_focus,
            );
        }
    });
    entry.add_controller(focus_controller);
}

fn remove_focus_controllers(entry: &gtk::Entry) {
    let controllers = entry.observe_controllers();
    let mut to_remove: Vec<gtk::EventController> = Vec::new();
    for index in 0..controllers.n_items() {
        let Some(object) = controllers.item(index) else {
            continue;
        };
        if object.downcast_ref::<gtk::EventControllerKey>().is_some()
            || object.downcast_ref::<gtk::EventControllerFocus>().is_some()
        {
            if let Ok(controller) = object.downcast::<gtk::EventController>() {
                to_remove.push(controller);
            }
        }
    }
    for controller in to_remove {
        entry.remove_controller(&controller);
    }
}

fn confirm_and_delete(
    anchor: &gtk::Widget,
    item: PlaylistItem,
    current_name: String,
    on_delete: DeleteCallbackHolder,
) {
    let Some(root) = anchor.root() else {
        return;
    };
    let Ok(parent_window) = root.downcast::<gtk::Window>() else {
        return;
    };

    let (title, detail, button_label) = match item {
        PlaylistItem::Folder(_) => (
            "Delete Folder",
            format!(
                "\"{current_name}\" will be deleted along with every playlist, smart playlist, and folder inside it. This cannot be undone."
            ),
            "Delete Folder",
        ),
        PlaylistItem::Playlist(_) => (
            "Delete Playlist",
            format!("\"{current_name}\" will be removed from the sidebar. The tracks themselves stay in your library."),
            "Delete Playlist",
        ),
        PlaylistItem::SmartPlaylist(_) => (
            "Delete Smart Playlist",
            format!("\"{current_name}\" will be removed from the sidebar. The tracks it currently matches stay in your library."),
            "Delete Smart Playlist",
        ),
    };

    let window = gtk::Window::builder()
        .title(title)
        .transient_for(&parent_window)
        .modal(true)
        .resizable(false)
        .default_width(440)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(18);
    content.set_margin_end(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);

    let detail_label = gtk::Label::new(Some(&detail));
    detail_label.add_css_class("dim-label");
    detail_label.set_xalign(0.0);
    detail_label.set_wrap(true);
    content.append(&detail_label);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);

    let cancel_button = gtk::Button::with_label("Cancel");
    let delete_button = gtk::Button::with_label(button_label);
    delete_button.add_css_class("destructive-action");

    let window_for_cancel = window.clone();
    cancel_button.connect_clicked(move |_| {
        window_for_cancel.close();
    });

    let window_for_delete = window.clone();
    delete_button.connect_clicked(move |_| {
        if let Some(callback) = on_delete.borrow().as_ref() {
            callback(item);
        }
        window_for_delete.close();
    });

    buttons.append(&cancel_button);
    buttons.append(&delete_button);
    content.append(&buttons);
    window.set_child(Some(&content));
    window.set_default_widget(Some(&cancel_button));

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

    window.present();
    cancel_button.grab_focus();
}

fn attach_drag_and_drop(
    row: &gtk::Box,
    item: PlaylistItem,
    on_move: MoveCallbackHolder,
    on_tracks_drop: TracksDropCallbackHolder,
) {
    remove_drag_and_drop_controllers(row);

    let drag_source = gtk::DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE);
    drag_source.connect_prepare(move |_source, _x, _y| {
        Some(gdk::ContentProvider::for_value(
            &drag_payload(item).to_value(),
        ))
    });
    row.add_controller(drag_source);

    let target_is_folder = matches!(item, PlaylistItem::Folder(_));
    let current_position: Rc<Cell<DropPosition>> = Rc::new(Cell::new(if target_is_folder {
        DropPosition::Into
    } else {
        DropPosition::Above
    }));

    let drop_target = gtk::DropTarget::new(
        glib::Type::STRING,
        gdk::DragAction::MOVE | gdk::DragAction::COPY,
    );

    let row_for_motion = row.clone();
    let current_position_for_motion = current_position.clone();
    drop_target.connect_motion(move |_target, _x, y| {
        let row_height = row_for_motion.height() as f64;
        let position = drop_position_from_motion(y, row_height, target_is_folder);
        if current_position_for_motion.get() != position {
            current_position_for_motion.set(position);
            set_drop_indicator(&row_for_motion, position);
        }
        gdk::DragAction::MOVE | gdk::DragAction::COPY
    });

    let row_for_leave = row.clone();
    drop_target.connect_leave(move |_target| {
        clear_drop_indicator(&row_for_leave);
    });

    let row_for_drop = row.clone();
    let current_position_for_drop = current_position.clone();
    drop_target.connect_drop(move |_target, value, _x, _y| {
        clear_drop_indicator(&row_for_drop);
        let position = current_position_for_drop.get();
        let Ok(text) = value.get::<String>() else {
            return false;
        };
        if let Some(track_ids) = parse_tracks_payload(&text) {
            if matches!(item, PlaylistItem::Playlist(_)) {
                if let Some(callback) = on_tracks_drop.borrow().as_ref() {
                    callback(item, track_ids);
                    return true;
                }
            }
            return false;
        }
        let Some(source_item) = parse_drag_payload(&text) else {
            return false;
        };
        if source_item == item {
            return false;
        }
        if let Some(callback) = on_move.borrow().as_ref() {
            callback(source_item, item, position);
        }
        true
    });
    row.add_controller(drop_target);
}

fn set_drop_indicator(row: &gtk::Box, position: DropPosition) {
    clear_drop_indicator(row);
    match position {
        DropPosition::Above => row.add_css_class("sidebar-drop-above"),
        DropPosition::Below => row.add_css_class("sidebar-drop-below"),
        DropPosition::Into => row.add_css_class("sidebar-drop-into"),
    }
}

fn clear_drop_indicator(row: &gtk::Box) {
    row.remove_css_class("sidebar-drop-above");
    row.remove_css_class("sidebar-drop-below");
    row.remove_css_class("sidebar-drop-into");
}

fn remove_drag_and_drop_controllers(widget: &gtk::Box) {
    let controllers = widget.observe_controllers();
    let mut to_remove: Vec<gtk::EventController> = Vec::new();
    for index in 0..controllers.n_items() {
        let Some(object) = controllers.item(index) else {
            continue;
        };
        let is_drag_or_drop = object.downcast_ref::<gtk::DragSource>().is_some()
            || object.downcast_ref::<gtk::DropTarget>().is_some();
        if !is_drag_or_drop {
            continue;
        }
        if let Ok(controller) = object.downcast::<gtk::EventController>() {
            to_remove.push(controller);
        }
    }
    for controller in to_remove {
        widget.remove_controller(&controller);
    }
}

fn drag_payload(item: PlaylistItem) -> String {
    match item {
        PlaylistItem::Folder(id) => format!("folder:{}", id.get()),
        PlaylistItem::Playlist(id) => format!("playlist:{}", id.get()),
        PlaylistItem::SmartPlaylist(id) => format!("smart:{}", id.get()),
    }
}

fn parse_drag_payload(text: &str) -> Option<PlaylistItem> {
    let (kind, id_str) = text.split_once(':')?;
    let id = id_str.parse::<i64>().ok()?;
    match kind {
        "folder" => PlaylistFolderId::new(id).map(PlaylistItem::Folder),
        "playlist" => PlaylistId::new(id).map(PlaylistItem::Playlist),
        "smart" => SmartPlaylistId::new(id).map(PlaylistItem::SmartPlaylist),
        _ => None,
    }
}

pub(crate) fn tracks_drag_payload(track_ids: &[TrackId]) -> String {
    let joined = track_ids
        .iter()
        .map(|id| id.get().to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("tracks:{joined}")
}

fn parse_tracks_payload(text: &str) -> Option<Vec<TrackId>> {
    let (kind, ids_str) = text.split_once(':')?;
    if kind != "tracks" {
        return None;
    }
    let ids: Option<Vec<TrackId>> = ids_str
        .split(',')
        .map(|part| part.trim().parse::<i64>().ok().and_then(TrackId::new))
        .collect();
    let ids = ids?;
    if ids.is_empty() { None } else { Some(ids) }
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

#[cfg(test)]
mod tests {
    use xtunes_app_runtime::{
        PlaylistId, SmartPlaylistId, SmartPlaylistMatchKind, SmartPlaylistRule,
        SmartPlaylistRuleSet, SmartPlaylistTextField, SmartPlaylistTextOperator,
    };

    use super::*;

    fn folder(id: i64, name: &str, parent: Option<PlaylistFolderId>, position: u32) -> PlaylistFolder {
        PlaylistFolder {
            id: PlaylistFolderId::new(id).expect("positive folder id"),
            name: name.to_owned(),
            parent_folder_id: parent,
            position,
        }
    }

    fn playlist(id: i64, name: &str, parent: Option<PlaylistFolderId>, position: u32) -> Playlist {
        Playlist {
            id: PlaylistId::new(id).expect("positive playlist id"),
            name: name.to_owned(),
            parent_folder_id: parent,
            position,
            entries: Vec::new(),
        }
    }

    fn smart_playlist(
        id: i64,
        name: &str,
        parent: Option<PlaylistFolderId>,
        position: u32,
    ) -> SmartPlaylist {
        SmartPlaylist {
            id: SmartPlaylistId::new(id).expect("positive smart playlist id"),
            name: name.to_owned(),
            parent_folder_id: parent,
            position,
            rules: SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::All,
                rules: vec![SmartPlaylistRule::Text {
                    field: SmartPlaylistTextField::Genre,
                    operator: SmartPlaylistTextOperator::Is,
                    value: "Trip-Hop".to_owned(),
                }],
                limit: None,
            },
        }
    }

    #[test]
    fn snapshot_groups_items_by_parent_and_orders_them_by_position() {
        let root_folder = folder(1, "Mixes", None, 1);
        let root_playlist = playlist(1, "Drive", None, 0);
        let root_smart = smart_playlist(1, "Recent", None, 2);
        let nested_playlist = playlist(2, "Inside", Some(root_folder.id), 0);

        let snapshot = SidebarSnapshot::build(
            &[root_folder.clone()],
            &[root_playlist.clone(), nested_playlist.clone()],
            &[root_smart.clone()],
        );

        let root_items: Vec<PlaylistItem> = snapshot
            .items_under(None)
            .iter()
            .map(|item| item.item)
            .collect();
        assert_eq!(
            root_items,
            vec![
                PlaylistItem::Playlist(root_playlist.id),
                PlaylistItem::Folder(root_folder.id),
                PlaylistItem::SmartPlaylist(root_smart.id),
            ]
        );

        let nested_items: Vec<PlaylistItem> = snapshot
            .items_under(Some(root_folder.id))
            .iter()
            .map(|item| item.item)
            .collect();
        assert_eq!(nested_items, vec![PlaylistItem::Playlist(nested_playlist.id)]);

        assert!(
            snapshot
                .items_under(Some(PlaylistFolderId::new(999).expect("positive id")))
                .is_empty()
        );
    }

    #[test]
    fn drag_payload_round_trips_for_each_kind() {
        let folder_id = PlaylistFolderId::new(42).expect("positive id");
        let playlist_id = PlaylistId::new(7).expect("positive id");
        let smart_id = SmartPlaylistId::new(3).expect("positive id");

        let cases = [
            PlaylistItem::Folder(folder_id),
            PlaylistItem::Playlist(playlist_id),
            PlaylistItem::SmartPlaylist(smart_id),
        ];
        for item in cases {
            let payload = drag_payload(item);
            assert_eq!(parse_drag_payload(&payload), Some(item));
        }
    }

    #[test]
    fn drag_payload_rejects_unknown_kind_and_invalid_id() {
        assert_eq!(parse_drag_payload("bogus:1"), None);
        assert_eq!(parse_drag_payload("folder:not-a-number"), None);
        assert_eq!(parse_drag_payload("folder:-3"), None);
        assert_eq!(parse_drag_payload("no-colon"), None);
    }

    #[test]
    fn drop_position_splits_non_folder_in_half() {
        assert_eq!(
            drop_position_from_motion(4.0, 20.0, false),
            DropPosition::Above
        );
        assert_eq!(
            drop_position_from_motion(15.0, 20.0, false),
            DropPosition::Below
        );
    }

    #[test]
    fn drop_position_uses_three_zones_for_folders() {
        assert_eq!(
            drop_position_from_motion(2.0, 20.0, true),
            DropPosition::Above
        );
        assert_eq!(
            drop_position_from_motion(10.0, 20.0, true),
            DropPosition::Into
        );
        assert_eq!(
            drop_position_from_motion(18.0, 20.0, true),
            DropPosition::Below
        );
    }

    #[test]
    fn tracks_payload_round_trips_for_multiple_ids() {
        let ids = vec![
            xtunes_app_runtime::TrackId::new(1).expect("positive"),
            xtunes_app_runtime::TrackId::new(7).expect("positive"),
            xtunes_app_runtime::TrackId::new(42).expect("positive"),
        ];
        let payload = tracks_drag_payload(&ids);
        assert_eq!(payload, "tracks:1,7,42");
        assert_eq!(parse_tracks_payload(&payload), Some(ids));
    }

    #[test]
    fn tracks_payload_rejects_malformed_input() {
        assert_eq!(parse_tracks_payload("tracks:"), None);
        assert_eq!(parse_tracks_payload("tracks:abc"), None);
        assert_eq!(parse_tracks_payload("tracks:-1"), None);
        assert_eq!(parse_tracks_payload("playlist:1"), None);
    }

    #[test]
    fn drop_position_handles_zero_height_gracefully() {
        assert_eq!(
            drop_position_from_motion(0.0, 0.0, false),
            DropPosition::Above
        );
        assert_eq!(
            drop_position_from_motion(0.0, 0.0, true),
            DropPosition::Into
        );
    }
}
