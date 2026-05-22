use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use xtunes_app_runtime::TrackId;

pub(crate) type TrackActionCallback = Rc<dyn Fn(Vec<TrackId>)>;
type PendingConfirmCallback = Rc<RefCell<Option<Box<dyn FnOnce(Vec<TrackId>)>>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrackContextActionId {
    RemoveFromLibrary,
    MoveToTrash,
}

impl TrackContextActionId {
    fn css_class(self) -> &'static str {
        match self {
            Self::RemoveFromLibrary => "track-context-remove-from-library",
            Self::MoveToTrash => "track-context-move-to-trash",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrackSelectionRequirement {
    AtLeastOne,
    Single,
}

const TRACK_SELECTION_REQUIREMENTS: &[TrackSelectionRequirement] = &[
    TrackSelectionRequirement::AtLeastOne,
    TrackSelectionRequirement::Single,
];

impl TrackSelectionRequirement {
    fn accepts(self, selected_count: usize) -> bool {
        match self {
            Self::AtLeastOne => selected_count > 0,
            Self::Single => selected_count == 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrackActionConfirmation {
    None,
    MoveToTrash,
}

#[derive(Clone)]
pub(crate) struct TrackContextAction {
    id: TrackContextActionId,
    label: &'static str,
    destructive: bool,
    selection: TrackSelectionRequirement,
    confirmation: TrackActionConfirmation,
    callback: TrackActionCallback,
}

impl TrackContextAction {
    pub(crate) fn remove_from_library(callback: TrackActionCallback) -> Self {
        Self {
            id: TrackContextActionId::RemoveFromLibrary,
            label: "Remove from Library",
            destructive: false,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::None,
            callback,
        }
    }

    pub(crate) fn move_to_trash(callback: TrackActionCallback) -> Self {
        Self {
            id: TrackContextActionId::MoveToTrash,
            label: "Move to Trash",
            destructive: true,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::MoveToTrash,
            callback,
        }
    }

    fn is_available(&self, selected_count: usize) -> bool {
        self.selection.accepts(selected_count)
    }
}

#[derive(Clone)]
pub(crate) struct TrackContextActionSet {
    actions: Vec<TrackContextAction>,
}

impl TrackContextActionSet {
    pub(crate) fn new(actions: Vec<TrackContextAction>) -> Self {
        debug_assert!(
            actions
                .iter()
                .all(|action| TRACK_SELECTION_REQUIREMENTS.contains(&action.selection))
        );
        Self { actions }
    }

    fn available_actions(
        &self,
        selected_count: usize,
    ) -> impl Iterator<Item = &TrackContextAction> {
        self.actions
            .iter()
            .filter(move |action| action.is_available(selected_count))
    }
}

#[derive(Clone)]
pub(crate) struct TrackRowContextMenu {
    actions: TrackContextActionSet,
    parent_window: gtk::Window,
}

impl TrackRowContextMenu {
    pub(crate) fn new(actions: TrackContextActionSet, parent_window: gtk::Window) -> Self {
        Self {
            actions,
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

        for action in self.actions.available_actions(track_ids.len()) {
            let button = context_menu_button(action);
            let action = action.clone();
            let parent = self.parent_window.clone();
            let popover = popover.clone();
            let track_ids = track_ids.clone();
            button.connect_clicked(move |_| {
                popover.popdown();
                run_context_action(&action, &parent, track_ids.clone());
            });
            content.append(&button);
        }

        content
    }
}

fn context_menu_button(action: &TrackContextAction) -> gtk::Button {
    let text = gtk::Label::new(Some(action.label));
    text.set_xalign(0.0);
    text.set_halign(gtk::Align::Fill);
    text.set_hexpand(true);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("track-context-menu-item");
    button.add_css_class(action.id.css_class());
    if action.destructive {
        button.add_css_class("destructive-action");
    }
    button.set_child(Some(&text));
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button
}

fn run_context_action(action: &TrackContextAction, parent: &gtk::Window, track_ids: Vec<TrackId>) {
    match action.confirmation {
        TrackActionConfirmation::None => {
            (action.callback)(track_ids);
        }
        TrackActionConfirmation::MoveToTrash => {
            let callback = action.callback.clone();
            confirm_move_to_trash(parent, track_ids, move |confirmed_ids| {
                callback(confirmed_ids);
            });
        }
    }
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
    use std::{cell::RefCell, rc::Rc};

    use super::{
        TrackActionCallback, TrackContextAction, TrackContextActionId, TrackSelectionRequirement,
        trash_confirmation_detail,
    };

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

    #[test]
    fn declared_actions_have_stable_identity_and_labels() {
        let callback = no_op_callback();
        let actions = [
            TrackContextAction::remove_from_library(callback.clone()),
            TrackContextAction::move_to_trash(callback),
        ];

        assert_eq!(actions[0].id, TrackContextActionId::RemoveFromLibrary);
        assert_eq!(actions[0].label, "Remove from Library");
        assert!(!actions[0].destructive);
        assert_eq!(actions[1].id, TrackContextActionId::MoveToTrash);
        assert_eq!(actions[1].label, "Move to Trash");
        assert!(actions[1].destructive);
    }

    #[test]
    fn action_selection_requirements_are_deterministic() {
        assert!(!TrackSelectionRequirement::AtLeastOne.accepts(0));
        assert!(TrackSelectionRequirement::AtLeastOne.accepts(2));
        assert!(TrackSelectionRequirement::Single.accepts(1));
        assert!(!TrackSelectionRequirement::Single.accepts(2));
    }

    fn no_op_callback() -> TrackActionCallback {
        Rc::new({
            let calls = Rc::new(RefCell::new(0usize));
            move |_track_ids| {
                *calls.borrow_mut() += 1;
            }
        })
    }
}
