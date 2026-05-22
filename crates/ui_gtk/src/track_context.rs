use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio};
use xtunes_app_runtime::TrackId;

pub(crate) type TrackActionCallback = Rc<dyn Fn(TrackId)>;

#[derive(Clone)]
pub(crate) struct TrackContextCallbacks {
    pub(crate) remove_from_library: TrackActionCallback,
    pub(crate) move_to_trash: TrackActionCallback,
}

const CONTEXT_GROUP: &str = "track-context";
const REMOVE_ACTION: &str = "remove-from-library";
const TRASH_ACTION: &str = "move-to-trash";

#[derive(Clone)]
pub(crate) struct TrackRowContextMenu {
    target_track_id: Rc<Cell<Option<TrackId>>>,
    menu_model: gio::Menu,
    action_group: gio::SimpleActionGroup,
}

impl TrackRowContextMenu {
    pub(crate) fn new(callbacks: TrackContextCallbacks) -> Self {
        let target_track_id: Rc<Cell<Option<TrackId>>> = Rc::new(Cell::new(None));

        let menu_model = gio::Menu::new();
        menu_model.append(
            Some("Remove from Library"),
            Some(&format!("{CONTEXT_GROUP}.{REMOVE_ACTION}")),
        );
        menu_model.append(
            Some("Move to Trash"),
            Some(&format!("{CONTEXT_GROUP}.{TRASH_ACTION}")),
        );

        let action_group = gio::SimpleActionGroup::new();

        let remove_action = gio::SimpleAction::new(REMOVE_ACTION, None);
        let target_for_remove = target_track_id.clone();
        let remove_callback = callbacks.remove_from_library;
        remove_action.connect_activate(move |_, _| {
            if let Some(track_id) = target_for_remove.get() {
                remove_callback(track_id);
            }
        });
        action_group.add_action(&remove_action);

        let trash_action = gio::SimpleAction::new(TRASH_ACTION, None);
        let target_for_trash = target_track_id.clone();
        let trash_callback = callbacks.move_to_trash;
        trash_action.connect_activate(move |_, _| {
            if let Some(track_id) = target_for_trash.get() {
                trash_callback(track_id);
            }
        });
        action_group.add_action(&trash_action);

        Self {
            target_track_id,
            menu_model,
            action_group,
        }
    }

    pub(crate) fn popup_at(
        &self,
        track_id: TrackId,
        anchor: &impl IsA<gtk::Widget>,
        x: f64,
        y: f64,
    ) {
        self.target_track_id.set(Some(track_id));

        let popover = gtk::PopoverMenu::from_model(Some(&self.menu_model));
        popover.set_has_arrow(false);
        popover.insert_action_group(CONTEXT_GROUP, Some(&self.action_group));
        popover.set_parent(anchor.as_ref());

        let popover_for_close = popover.clone();
        popover.connect_closed(move |_| {
            popover_for_close.unparent();
        });

        let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
        popover.popup();
    }
}
