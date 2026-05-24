// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use sustain_app_runtime::{PlaylistId, TrackId};

pub(crate) type TrackActionCallback = Rc<dyn Fn(Vec<TrackId>)>;
pub(crate) type TrackActionVisibility = Rc<dyn Fn(&[TrackId]) -> bool>;
pub(crate) type AddToPlaylistProvider = Rc<dyn Fn() -> Vec<AddToPlaylistEntry>>;
pub(crate) type AddToPlaylistCallback = Rc<dyn Fn(PlaylistId, Vec<TrackId>)>;
type PendingConfirmCallback = Rc<RefCell<Option<Box<dyn FnOnce(Vec<TrackId>)>>>>;

#[derive(Clone, Debug)]
pub(crate) struct AddToPlaylistEntry {
    pub playlist_id: PlaylistId,
    pub display_path: String,
}

#[derive(Clone)]
struct AddToPlaylistAction {
    provider: AddToPlaylistProvider,
    callback: AddToPlaylistCallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrackContextActionId {
    PlayNext,
    GetInfo,
    CopyFiles,
    ShowInFolder,
    ShowAlbum,
    RemoveFromLibrary,
    MoveToTrash,
    RemoveFromPlaylist,
}

impl TrackContextActionId {
    fn css_class(self) -> &'static str {
        match self {
            Self::PlayNext => "track-context-play-next",
            Self::GetInfo => "track-context-get-info",
            Self::CopyFiles => "track-context-copy-files",
            Self::ShowInFolder => "track-context-show-in-folder",
            Self::ShowAlbum => "track-context-show-album",
            Self::RemoveFromLibrary => "track-context-remove-from-library",
            Self::MoveToTrash => "track-context-move-to-trash",
            Self::RemoveFromPlaylist => "track-context-remove-from-playlist",
        }
    }
}

/// Visual grouping inside the popover. Safe actions render above the separator
/// next to "Add to Playlist"; destructive actions render below it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrackContextActionSection {
    Safe,
    Destructive,
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
    section: TrackContextActionSection,
    selection: TrackSelectionRequirement,
    confirmation: TrackActionConfirmation,
    /// When `Some`, the predicate is evaluated each time the menu is popped
    /// and the action is hidden if it returns `false`. Used for actions that
    /// only make sense in a specific view (e.g. Remove from Playlist only
    /// when a regular playlist is currently selected in the sidebar).
    visibility: Option<TrackActionVisibility>,
    callback: TrackActionCallback,
}

impl TrackContextAction {
    pub(crate) fn play_next(
        callback: TrackActionCallback,
        visibility: TrackActionVisibility,
    ) -> Self {
        Self {
            id: TrackContextActionId::PlayNext,
            label: "Play Next",
            section: TrackContextActionSection::Safe,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::None,
            visibility: Some(visibility),
            callback,
        }
    }

    pub(crate) fn get_info(callback: TrackActionCallback) -> Self {
        Self {
            id: TrackContextActionId::GetInfo,
            label: "Get Info",
            section: TrackContextActionSection::Safe,
            selection: TrackSelectionRequirement::Single,
            confirmation: TrackActionConfirmation::None,
            visibility: None,
            callback,
        }
    }

    pub(crate) fn copy_files(callback: TrackActionCallback) -> Self {
        Self {
            id: TrackContextActionId::CopyFiles,
            label: "Copy",
            section: TrackContextActionSection::Safe,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::None,
            visibility: None,
            callback,
        }
    }

    pub(crate) fn show_in_folder(callback: TrackActionCallback) -> Self {
        Self {
            id: TrackContextActionId::ShowInFolder,
            label: "Show in Folder",
            section: TrackContextActionSection::Safe,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::None,
            visibility: None,
            callback,
        }
    }

    pub(crate) fn show_album(
        callback: TrackActionCallback,
        visibility: TrackActionVisibility,
    ) -> Self {
        Self {
            id: TrackContextActionId::ShowAlbum,
            label: "Show Album",
            section: TrackContextActionSection::Safe,
            selection: TrackSelectionRequirement::Single,
            confirmation: TrackActionConfirmation::None,
            visibility: Some(visibility),
            callback,
        }
    }

    pub(crate) fn remove_from_library(callback: TrackActionCallback) -> Self {
        Self {
            id: TrackContextActionId::RemoveFromLibrary,
            label: "Remove from Library",
            section: TrackContextActionSection::Destructive,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::None,
            visibility: None,
            callback,
        }
    }

    pub(crate) fn move_to_trash(callback: TrackActionCallback) -> Self {
        Self {
            id: TrackContextActionId::MoveToTrash,
            label: "Move to Trash",
            section: TrackContextActionSection::Destructive,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::MoveToTrash,
            visibility: None,
            callback,
        }
    }

    pub(crate) fn remove_from_playlist(
        callback: TrackActionCallback,
        visibility: TrackActionVisibility,
    ) -> Self {
        Self {
            id: TrackContextActionId::RemoveFromPlaylist,
            label: "Remove from Playlist",
            section: TrackContextActionSection::Destructive,
            selection: TrackSelectionRequirement::AtLeastOne,
            confirmation: TrackActionConfirmation::None,
            visibility: Some(visibility),
            callback,
        }
    }

    fn is_available(&self, track_ids: &[TrackId]) -> bool {
        if !self.selection.accepts(track_ids.len()) {
            return false;
        }
        if let Some(predicate) = &self.visibility
            && !predicate(track_ids)
        {
            return false;
        }
        true
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

    fn available_actions<'a>(
        &'a self,
        track_ids: &'a [TrackId],
    ) -> impl Iterator<Item = &'a TrackContextAction> {
        self.actions
            .iter()
            .filter(move |action| action.is_available(track_ids))
    }
}

#[derive(Clone)]
pub(crate) struct TrackRowContextMenu {
    actions: TrackContextActionSet,
    parent_window: gtk::Window,
    add_to_playlist: Option<AddToPlaylistAction>,
}

impl TrackRowContextMenu {
    pub(crate) fn new(actions: TrackContextActionSet, parent_window: gtk::Window) -> Self {
        Self {
            actions,
            parent_window,
            add_to_playlist: None,
        }
    }

    pub(crate) fn with_add_to_playlist(
        mut self,
        provider: AddToPlaylistProvider,
        callback: AddToPlaylistCallback,
    ) -> Self {
        self.add_to_playlist = Some(AddToPlaylistAction { provider, callback });
        self
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
            let Some(point) = anchor.as_ref().compute_point(
                popover_parent.as_ref(),
                &gtk::graphene::Point::new(x as f32, y as f32),
            ) else {
                return;
            };
            (point.x() as f64, point.y() as f64)
        };

        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        popover.add_css_class("compact-context-menu");
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
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(140);

        stack.add_named(
            &self.build_main_page(popover, &stack, track_ids.clone()),
            Some("main"),
        );
        if let Some(add) = &self.add_to_playlist {
            stack.add_named(
                &build_add_to_playlist_page(add, popover, &stack, track_ids.clone()),
                Some("playlists"),
            );
        }
        stack.set_visible_child_name("main");

        outer.append(&stack);
        outer
    }

    fn build_main_page(
        &self,
        popover: &gtk::Popover,
        stack: &gtk::Stack,
        track_ids: Vec<TrackId>,
    ) -> gtk::Box {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("track-context-menu");

        let mut has_safe_section = false;
        if self.add_to_playlist.is_some() {
            let add_button = submenu_button("Add to Playlist\u{2026}");
            let stack_for_add = stack.clone();
            add_button.connect_clicked(move |_| {
                stack_for_add.set_visible_child_name("playlists");
            });
            content.append(&add_button);
            has_safe_section = true;
        }

        let available: Vec<&TrackContextAction> =
            self.actions.available_actions(&track_ids).collect();
        let (safe, destructive): (Vec<&TrackContextAction>, Vec<&TrackContextAction>) = available
            .into_iter()
            .partition(|action| action.section == TrackContextActionSection::Safe);

        for action in &safe {
            self.append_action_button(&content, popover, action, &track_ids);
            has_safe_section = true;
        }

        if has_safe_section && !destructive.is_empty() {
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            separator.add_css_class("track-context-menu-separator");
            content.append(&separator);
        }

        for action in &destructive {
            self.append_action_button(&content, popover, action, &track_ids);
        }

        content
    }

    fn append_action_button(
        &self,
        content: &gtk::Box,
        popover: &gtk::Popover,
        action: &TrackContextAction,
        track_ids: &[TrackId],
    ) {
        let button = context_menu_button(action);
        let action = action.clone();
        let parent = self.parent_window.clone();
        let popover = popover.clone();
        let track_ids = track_ids.to_vec();
        button.connect_clicked(move |_| {
            popover.popdown();
            run_context_action(&action, &parent, track_ids.clone());
        });
        content.append(&button);
    }
}

fn build_add_to_playlist_page(
    action: &AddToPlaylistAction,
    popover: &gtk::Popover,
    stack: &gtk::Stack,
    track_ids: Vec<TrackId>,
) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("track-context-menu");
    content.add_css_class("track-context-submenu");

    let back_button = submenu_back_button("Back");
    let stack_for_back = stack.clone();
    back_button.connect_clicked(move |_| {
        stack_for_back.set_visible_child_name("main");
    });
    content.append(&back_button);

    let entries = (action.provider)();
    if entries.is_empty() {
        let empty_label = gtk::Label::new(Some("No playlists."));
        empty_label.set_xalign(0.0);
        empty_label.add_css_class("dim-label");
        empty_label.set_margin_top(6);
        empty_label.set_margin_bottom(6);
        empty_label.set_margin_start(8);
        empty_label.set_margin_end(8);
        content.append(&empty_label);
    } else {
        for entry in entries {
            let button = context_menu_button_with_label(&entry.display_path);
            let callback = action.callback.clone();
            let popover_for_pick = popover.clone();
            let track_ids = track_ids.clone();
            let playlist_id = entry.playlist_id;
            button.connect_clicked(move |_| {
                popover_for_pick.popdown();
                callback(playlist_id, track_ids.clone());
            });
            content.append(&button);
        }
    }

    content
}

fn submenu_button(label_text: &str) -> gtk::Button {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Start);
    label.set_hexpand(true);

    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.set_pixel_size(12);
    chevron.add_css_class("track-context-submenu-chevron");

    let box_widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    box_widget.append(&label);
    box_widget.append(&chevron);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("track-context-menu-item");
    button.set_child(Some(&box_widget));
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button
}

fn submenu_back_button(label_text: &str) -> gtk::Button {
    let caret = gtk::Image::from_icon_name("go-previous-symbolic");
    caret.set_pixel_size(12);
    caret.add_css_class("track-context-submenu-back-caret");

    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Start);
    label.set_hexpand(true);
    label.set_margin_start(6);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&caret);
    row.append(&label);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("track-context-menu-item");
    button.add_css_class("track-context-submenu-back");
    button.set_child(Some(&row));
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button
}

fn context_menu_button_with_label(label_text: &str) -> gtk::Button {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Fill);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("track-context-menu-item");
    button.set_child(Some(&label));
    button.set_halign(gtk::Align::Fill);
    button.set_hexpand(true);
    button
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

    use sustain_app_runtime::TrackId;

    use super::{
        TrackActionCallback, TrackActionVisibility, TrackContextAction, TrackContextActionId,
        TrackContextActionSection, TrackSelectionRequirement, trash_confirmation_detail,
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
            TrackContextAction::play_next(callback.clone(), always_visible()),
            TrackContextAction::get_info(callback.clone()),
            TrackContextAction::copy_files(callback.clone()),
            TrackContextAction::show_in_folder(callback.clone()),
            TrackContextAction::show_album(callback.clone(), always_visible()),
            TrackContextAction::remove_from_library(callback.clone()),
            TrackContextAction::move_to_trash(callback),
        ];

        assert_eq!(actions[0].id, TrackContextActionId::PlayNext);
        assert_eq!(actions[0].label, "Play Next");
        assert_eq!(actions[1].id, TrackContextActionId::GetInfo);
        assert_eq!(actions[1].label, "Get Info");
        assert_eq!(actions[2].id, TrackContextActionId::CopyFiles);
        assert_eq!(actions[2].label, "Copy");
        assert_eq!(actions[3].id, TrackContextActionId::ShowInFolder);
        assert_eq!(actions[3].label, "Show in Folder");
        assert_eq!(actions[4].id, TrackContextActionId::ShowAlbum);
        assert_eq!(actions[4].label, "Show Album");
        assert_eq!(actions[5].id, TrackContextActionId::RemoveFromLibrary);
        assert_eq!(actions[5].label, "Remove from Library");
        assert_eq!(actions[6].id, TrackContextActionId::MoveToTrash);
        assert_eq!(actions[6].label, "Move to Trash");
    }

    #[test]
    fn get_info_requires_single_selection() {
        let action = TrackContextAction::get_info(no_op_callback());
        let one = TrackId::new(1).expect("positive track id");
        let two = TrackId::new(2).expect("positive track id");

        assert!(!action.is_available(&[]));
        assert!(action.is_available(&[one]));
        assert!(!action.is_available(&[one, two]));
    }

    #[test]
    fn safe_actions_render_above_destructive_ones() {
        let callback = no_op_callback();
        assert_eq!(
            TrackContextAction::play_next(callback.clone(), always_visible()).section,
            TrackContextActionSection::Safe,
        );
        assert_eq!(
            TrackContextAction::copy_files(callback.clone()).section,
            TrackContextActionSection::Safe,
        );
        assert_eq!(
            TrackContextAction::show_in_folder(callback.clone()).section,
            TrackContextActionSection::Safe,
        );
        assert_eq!(
            TrackContextAction::show_album(callback.clone(), always_visible()).section,
            TrackContextActionSection::Safe,
        );
        assert_eq!(
            TrackContextAction::remove_from_library(callback.clone()).section,
            TrackContextActionSection::Destructive,
        );
        assert_eq!(
            TrackContextAction::move_to_trash(callback).section,
            TrackContextActionSection::Destructive,
        );
    }

    #[test]
    fn play_next_is_hidden_when_visibility_predicate_returns_false() {
        let callback = no_op_callback();
        let track_id = TrackId::new(1).expect("positive track id");

        let visible = TrackContextAction::play_next(callback.clone(), always_visible());
        assert!(visible.is_available(&[track_id]));

        let hidden = TrackContextAction::play_next(callback, never_visible());
        assert!(!hidden.is_available(&[track_id]));
    }

    #[test]
    fn show_album_is_hidden_when_visibility_predicate_returns_false() {
        let callback = no_op_callback();
        let track_id = TrackId::new(1).expect("positive track id");

        let visible = TrackContextAction::show_album(callback.clone(), always_visible());
        assert!(visible.is_available(&[track_id]));

        let hidden = TrackContextAction::show_album(callback, never_visible());
        assert!(!hidden.is_available(&[track_id]));
    }

    #[test]
    fn show_album_requires_single_selection() {
        let action = TrackContextAction::show_album(no_op_callback(), always_visible());
        let one = TrackId::new(1).expect("positive track id");
        let two = TrackId::new(2).expect("positive track id");

        assert!(!action.is_available(&[]));
        assert!(action.is_available(&[one]));
        assert!(!action.is_available(&[one, two]));
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

    fn always_visible() -> TrackActionVisibility {
        Rc::new(|_track_ids| true)
    }

    fn never_visible() -> TrackActionVisibility {
        Rc::new(|_track_ids| false)
    }
}
