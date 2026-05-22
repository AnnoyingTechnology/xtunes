use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use xtunes_app_runtime::TrackId;

pub(crate) type TrackActionCallback = Rc<dyn Fn(Vec<TrackId>)>;

#[derive(Clone)]
pub(crate) struct TrackContextCallbacks {
    pub(crate) remove_from_library: TrackActionCallback,
    pub(crate) move_to_trash: TrackActionCallback,
}

const CONTEXT_GROUP: &str = "track-context";
const REMOVE_ACTION: &str = "remove-from-library";
const TRASH_ACTION: &str = "move-to-trash";
const CANCEL_BUTTON_INDEX: i32 = 0;
const CONFIRM_BUTTON_INDEX: i32 = 1;

#[derive(Clone)]
pub(crate) struct TrackRowContextMenu {
    target_track_ids: Rc<RefCell<Vec<TrackId>>>,
    menu_model: gio::Menu,
    action_group: gio::SimpleActionGroup,
}

impl TrackRowContextMenu {
    pub(crate) fn new(
        callbacks: TrackContextCallbacks,
        parent_window: gtk::Window,
    ) -> Self {
        let target_track_ids: Rc<RefCell<Vec<TrackId>>> = Rc::new(RefCell::new(Vec::new()));

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
        let target_for_remove = target_track_ids.clone();
        let remove_callback = callbacks.remove_from_library;
        remove_action.connect_activate(move |_, _| {
            let ids = target_for_remove.borrow().clone();
            if ids.is_empty() {
                return;
            }
            remove_callback(ids);
        });
        action_group.add_action(&remove_action);

        let trash_action = gio::SimpleAction::new(TRASH_ACTION, None);
        let target_for_trash = target_track_ids.clone();
        let window_for_trash = parent_window;
        let trash_callback = callbacks.move_to_trash;
        trash_action.connect_activate(move |_, _| {
            let ids = target_for_trash.borrow().clone();
            eprintln!(
                "[xtunes][ctxmenu] trash action fired with {} ids",
                ids.len()
            );
            if ids.is_empty() {
                return;
            }

            // Defer the dialog to the next idle so the popover has finished
            // closing before the modal opens.
            let parent = window_for_trash.clone();
            let trash_callback = trash_callback.clone();
            glib::idle_add_local_once(move || {
                eprintln!("[xtunes][ctxmenu] idle fired — building dialog");
                confirm_move_to_trash(&parent, ids, move |confirmed_ids| {
                    eprintln!(
                        "[xtunes][ctxmenu] on_confirm running ({} ids)",
                        confirmed_ids.len()
                    );
                    trash_callback(confirmed_ids);
                });
            });
        });
        action_group.add_action(&trash_action);

        Self {
            target_track_ids,
            menu_model,
            action_group,
        }
    }

    pub(crate) fn popup_at(
        &self,
        track_ids: Vec<TrackId>,
        anchor: &impl IsA<gtk::Widget>,
        x: f64,
        y: f64,
    ) {
        if track_ids.is_empty() {
            return;
        }

        self.target_track_ids.replace(track_ids);

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

fn confirm_move_to_trash(
    parent: &gtk::Window,
    track_ids: Vec<TrackId>,
    on_confirm: impl FnOnce(Vec<TrackId>) + 'static,
) {
    let (message, detail) = trash_confirmation_text(track_ids.len());
    eprintln!(
        "[xtunes][ctxmenu] confirm_move_to_trash: parent={:?}, message={}",
        parent.title().map(|t| t.to_string()),
        message
    );

    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(message)
        .detail(detail)
        .buttons(["Cancel", "Move to Trash"])
        .default_button(CONFIRM_BUTTON_INDEX)
        .cancel_button(CANCEL_BUTTON_INDEX)
        .build();

    // Hold the dialog alive in the callback so the GObject cannot be freed
    // before the async choose completes.
    let dialog_keepalive = dialog.clone();
    dialog.choose(
        Some(parent),
        None::<&gio::Cancellable>,
        move |result| {
            eprintln!(
                "[xtunes][ctxmenu] AlertDialog.choose callback fired with result: {:?}",
                result
            );
            let _keepalive = dialog_keepalive;
            if matches!(result, Ok(index) if index == CONFIRM_BUTTON_INDEX) {
                on_confirm(track_ids);
            }
        },
    );
    eprintln!("[xtunes][ctxmenu] dialog.choose returned (async pending)");
}

fn trash_confirmation_text(count: usize) -> (String, String) {
    if count == 1 {
        (
            "Move this track to the Trash?".to_owned(),
            "The audio file will be moved to the system trash and the track will be removed from the library.".to_owned(),
        )
    } else {
        (
            format!("Move {count} tracks to the Trash?"),
            format!(
                "The {count} audio files will be moved to the system trash and the tracks will be removed from the library."
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::trash_confirmation_text;

    #[test]
    fn single_track_confirmation_uses_singular_phrasing() {
        let (message, detail) = trash_confirmation_text(1);
        assert_eq!(message, "Move this track to the Trash?");
        assert!(detail.contains("audio file will be moved"));
    }

    #[test]
    fn multi_track_confirmation_uses_plural_phrasing_with_count() {
        let (message, detail) = trash_confirmation_text(3);
        assert_eq!(message, "Move 3 tracks to the Trash?");
        assert!(detail.contains("3 audio files"));
    }
}
