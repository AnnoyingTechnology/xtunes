use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use xtunes_app_runtime::TrackId;

pub(crate) type TrackActionCallback = Rc<dyn Fn(Vec<TrackId>)>;
type PendingConfirmCallback = Rc<RefCell<Option<Box<dyn FnOnce(Vec<TrackId>)>>>>;

#[derive(Clone)]
pub(crate) struct TrackContextCallbacks {
    pub(crate) remove_from_library: TrackActionCallback,
    pub(crate) move_to_trash: TrackActionCallback,
}

#[derive(Clone)]
pub(crate) struct TrackRowContextMenu {
    callbacks: TrackContextCallbacks,
    parent_window: gtk::Window,
}

impl TrackRowContextMenu {
    pub(crate) fn new(callbacks: TrackContextCallbacks, parent_window: gtk::Window) -> Self {
        Self {
            callbacks,
            parent_window,
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

        self.popup_at_parent(track_ids, anchor, anchor, x, y);
    }

    pub(crate) fn popup_at_parent(
        &self,
        track_ids: Vec<TrackId>,
        anchor: &impl IsA<gtk::Widget>,
        popover_parent: &impl IsA<gtk::Widget>,
        x: f64,
        y: f64,
    ) {
        if track_ids.is_empty() {
            return;
        }

        let (parent_x, parent_y) = if anchor.as_ref() == popover_parent.as_ref() {
            (x, y)
        } else {
            let Some(coordinates) =
                anchor
                    .as_ref()
                    .translate_coordinates(popover_parent.as_ref(), x, y)
            else {
                return;
            };
            coordinates
        };

        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        popover.set_parent(popover_parent.as_ref());
        popover.set_child(Some(&self.menu_content(&popover, track_ids)));

        let popover_for_close = popover.clone();
        popover.connect_closed(move |_| {
            popover_for_close.unparent();
        });

        let rect = gdk::Rectangle::new(parent_x as i32, parent_y as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
        popover.popup();
    }

    fn menu_content(&self, popover: &gtk::Popover, track_ids: Vec<TrackId>) -> gtk::Box {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("track-context-menu");

        let remove_button = context_menu_button("Remove from Library");
        let ids_for_remove = track_ids.clone();
        let remove_callback = self.callbacks.remove_from_library.clone();
        let popover_for_remove = popover.clone();
        remove_button.connect_clicked(move |_| {
            popover_for_remove.popdown();
            remove_callback(ids_for_remove.clone());
        });
        content.append(&remove_button);

        let trash_button = context_menu_button("Move to Trash");
        let parent = self.parent_window.clone();
        let trash_callback = self.callbacks.move_to_trash.clone();
        let popover_for_trash = popover.clone();
        trash_button.connect_clicked(move |_| {
            popover_for_trash.popdown();
            confirm_move_to_trash(&parent, track_ids.clone(), {
                let trash_callback = trash_callback.clone();
                move |confirmed_ids| trash_callback(confirmed_ids)
            });
        });
        content.append(&trash_button);

        content
    }
}

fn context_menu_button(label: &str) -> gtk::Button {
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    text.set_halign(gtk::Align::Fill);
    text.set_hexpand(true);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("track-context-menu-item");
    button.set_child(Some(&text));
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button
}

fn confirm_move_to_trash(
    parent: &gtk::Window,
    track_ids: Vec<TrackId>,
    on_confirm: impl FnOnce(Vec<TrackId>) + 'static,
) {
    let detail = trash_confirmation_detail(track_ids.len());

    let window = gtk::Window::builder()
        .title("Move to Trash")
        .transient_for(parent)
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
    let trash_button = gtk::Button::with_label("Move to Trash");
    trash_button.add_css_class("destructive-action");

    let window_for_cancel = window.clone();
    cancel_button.connect_clicked(move |_| {
        window_for_cancel.close();
    });

    let confirm_callback: PendingConfirmCallback =
        Rc::new(RefCell::new(Some(Box::new(on_confirm))));
    let callback_for_trash = confirm_callback.clone();
    let window_for_trash = window.clone();
    trash_button.connect_clicked(move |_| {
        if let Some(callback) = callback_for_trash.borrow_mut().take() {
            callback(track_ids.clone());
        }
        window_for_trash.close();
    });

    buttons.append(&cancel_button);
    buttons.append(&trash_button);
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

fn trash_confirmation_detail(count: usize) -> String {
    if count == 1 {
        "The audio file will be moved to the system trash and the track will be removed from the library.".to_owned()
    } else {
        format!(
            "The {count} audio files will be moved to the system trash and the tracks will be removed from the library."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::trash_confirmation_detail;

    #[test]
    fn single_track_confirmation_detail_uses_singular_phrasing() {
        let detail = trash_confirmation_detail(1);
        assert!(detail.contains("audio file will be moved"));
    }

    #[test]
    fn multi_track_confirmation_detail_uses_plural_phrasing_with_count() {
        let detail = trash_confirmation_detail(3);
        assert!(detail.contains("3 audio files"));
    }
}
