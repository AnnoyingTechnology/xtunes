// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{cell::RefCell, num::NonZeroU32, rc::Rc};

use gtk::prelude::*;
use gtk::{gdk, glib};

use xtunes_app_runtime::{
    ApplicationCommand, SmartPlaylistLimit, SmartPlaylistLimitSelection, SmartPlaylistMatchKind,
    SmartPlaylistNumberField, SmartPlaylistNumberOperator, SmartPlaylistRule,
    SmartPlaylistRuleSet, SmartPlaylistTextField, SmartPlaylistTextOperator,
};

use super::{
    SMART_PLAYLIST_EDITOR_HEIGHT, SMART_PLAYLIST_EDITOR_WIDTH, WINDOW_SHADOW_MARGIN,
    command_controller::SharedCommandController,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorField {
    Text(SmartPlaylistTextField),
    Number(SmartPlaylistNumberField),
}

const EDITOR_FIELDS: &[(EditorField, &str)] = &[
    (EditorField::Text(SmartPlaylistTextField::Artist), "Artist"),
    (
        EditorField::Text(SmartPlaylistTextField::AlbumArtist),
        "Album Artist",
    ),
    (EditorField::Text(SmartPlaylistTextField::Album), "Album"),
    (EditorField::Text(SmartPlaylistTextField::Title), "Title"),
    (
        EditorField::Text(SmartPlaylistTextField::Composer),
        "Composer",
    ),
    (EditorField::Text(SmartPlaylistTextField::Genre), "Genre"),
    (EditorField::Number(SmartPlaylistNumberField::Year), "Year"),
    (
        EditorField::Number(SmartPlaylistNumberField::PlayCount),
        "Play Count",
    ),
    (
        EditorField::Number(SmartPlaylistNumberField::SkipCount),
        "Skip Count",
    ),
    (
        EditorField::Number(SmartPlaylistNumberField::TrackNumber),
        "Track Number",
    ),
    (
        EditorField::Number(SmartPlaylistNumberField::DiscNumber),
        "Disc Number",
    ),
    (
        EditorField::Number(SmartPlaylistNumberField::DurationSeconds),
        "Duration (seconds)",
    ),
    (
        EditorField::Number(SmartPlaylistNumberField::BitrateKbps),
        "Bitrate (kbps)",
    ),
];

const TEXT_OPERATORS: &[(SmartPlaylistTextOperator, &str)] = &[
    (SmartPlaylistTextOperator::Contains, "contains"),
    (SmartPlaylistTextOperator::DoesNotContain, "does not contain"),
    (SmartPlaylistTextOperator::Is, "is"),
    (SmartPlaylistTextOperator::IsNot, "is not"),
    (SmartPlaylistTextOperator::StartsWith, "starts with"),
    (SmartPlaylistTextOperator::EndsWith, "ends with"),
];

const NUMBER_OPERATORS: &[(SmartPlaylistNumberOperator, &str)] = &[
    (SmartPlaylistNumberOperator::Equal, "is"),
    (SmartPlaylistNumberOperator::NotEqual, "is not"),
    (SmartPlaylistNumberOperator::GreaterThan, "is greater than"),
    (
        SmartPlaylistNumberOperator::GreaterThanOrEqual,
        "is greater than or equal to",
    ),
    (SmartPlaylistNumberOperator::LessThan, "is less than"),
    (
        SmartPlaylistNumberOperator::LessThanOrEqual,
        "is less than or equal to",
    ),
];

const MATCH_KINDS: &[(SmartPlaylistMatchKind, &str)] = &[
    (SmartPlaylistMatchKind::All, "all"),
    (SmartPlaylistMatchKind::Any, "any"),
];

const LIMIT_SELECTIONS: &[(SmartPlaylistLimitSelection, &str)] = &[
    (SmartPlaylistLimitSelection::Random, "random"),
    (
        SmartPlaylistLimitSelection::MostRecentlyAdded,
        "most recently added",
    ),
    (
        SmartPlaylistLimitSelection::LeastRecentlyAdded,
        "least recently added",
    ),
    (
        SmartPlaylistLimitSelection::MostRecentlyPlayed,
        "most recently played",
    ),
    (
        SmartPlaylistLimitSelection::LeastRecentlyPlayed,
        "least recently played",
    ),
    (
        SmartPlaylistLimitSelection::MostOftenPlayed,
        "most often played",
    ),
    (
        SmartPlaylistLimitSelection::LeastOftenPlayed,
        "least often played",
    ),
    (SmartPlaylistLimitSelection::HighestRating, "highest rating"),
    (SmartPlaylistLimitSelection::LowestRating, "lowest rating"),
    (SmartPlaylistLimitSelection::TitleAscending, "title"),
    (SmartPlaylistLimitSelection::ArtistAscending, "artist"),
    (SmartPlaylistLimitSelection::AlbumAscending, "album"),
    (SmartPlaylistLimitSelection::GenreAscending, "genre"),
];

#[derive(Clone)]
struct RuleRow {
    container: gtk::Box,
    field_combo: gtk::DropDown,
    operator_combo: gtk::DropDown,
    value_entry: gtk::Entry,
}

impl RuleRow {
    fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        container.add_css_class("smart-playlist-rule-row");

        let field_model = gtk::StringList::new(
            &EDITOR_FIELDS
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>(),
        );
        let field_combo = gtk::DropDown::new(Some(field_model), gtk::Expression::NONE);
        field_combo.set_selected(0);

        let operator_combo = gtk::DropDown::new(Some(operator_model_for(EDITOR_FIELDS[0].0)), gtk::Expression::NONE);
        operator_combo.set_selected(0);

        let value_entry = gtk::Entry::new();
        value_entry.set_hexpand(true);
        value_entry.set_placeholder_text(Some("value"));

        let operator_combo_clone = operator_combo.clone();
        field_combo.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected() as usize;
            let Some((field, _)) = EDITOR_FIELDS.get(index) else {
                return;
            };
            operator_combo_clone.set_model(Some(&operator_model_for(*field)));
            operator_combo_clone.set_selected(0);
        });

        container.append(&field_combo);
        container.append(&operator_combo);
        container.append(&value_entry);

        Self {
            container,
            field_combo,
            operator_combo,
            value_entry,
        }
    }

    fn extract(&self) -> Result<SmartPlaylistRule, RuleError> {
        extract_rule(
            self.field_combo.selected() as usize,
            self.operator_combo.selected() as usize,
            &self.value_entry.text(),
        )
    }
}

fn extract_rule(
    field_index: usize,
    operator_index: usize,
    raw_value: &str,
) -> Result<SmartPlaylistRule, RuleError> {
    let (field, _) = EDITOR_FIELDS
        .get(field_index)
        .copied()
        .ok_or(RuleError::FieldMissing)?;

    match field {
        EditorField::Text(text_field) => {
            let (operator, _) = TEXT_OPERATORS
                .get(operator_index)
                .copied()
                .ok_or(RuleError::OperatorMissing)?;
            let value = raw_value.trim().to_owned();
            if value.is_empty() {
                return Err(RuleError::EmptyTextValue);
            }
            Ok(SmartPlaylistRule::Text {
                field: text_field,
                operator,
                value,
            })
        }
        EditorField::Number(number_field) => {
            let (operator, _) = NUMBER_OPERATORS
                .get(operator_index)
                .copied()
                .ok_or(RuleError::OperatorMissing)?;
            let trimmed = raw_value.trim();
            if trimmed.is_empty() {
                return Err(RuleError::EmptyNumberValue);
            }
            let value = trimmed
                .parse::<i64>()
                .map_err(|_| RuleError::NonNumericValue {
                    offending: trimmed.to_owned(),
                })?;
            Ok(SmartPlaylistRule::Number {
                field: number_field,
                operator,
                value,
            })
        }
    }
}

fn operator_model_for(field: EditorField) -> gtk::StringList {
    match field {
        EditorField::Text(_) => gtk::StringList::new(
            &TEXT_OPERATORS
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>(),
        ),
        EditorField::Number(_) => gtk::StringList::new(
            &NUMBER_OPERATORS
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>(),
        ),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RuleError {
    FieldMissing,
    OperatorMissing,
    EmptyTextValue,
    EmptyNumberValue,
    NonNumericValue { offending: String },
}

impl RuleError {
    fn message(&self) -> String {
        match self {
            Self::FieldMissing | Self::OperatorMissing => {
                "A rule is missing a field or operator.".to_owned()
            }
            Self::EmptyTextValue => "A text rule needs a value.".to_owned(),
            Self::EmptyNumberValue => "A numeric rule needs a value.".to_owned(),
            Self::NonNumericValue { offending } => {
                format!("\"{offending}\" is not a valid number.")
            }
        }
    }
}

pub(crate) fn open_smart_playlist_editor(
    parent: &gtk::ApplicationWindow,
    command_controller: SharedCommandController,
    on_created: Rc<dyn Fn()>,
    default_name: String,
) {
    let window = gtk::Window::builder()
        .title("Smart Playlist")
        .decorated(false)
        .transient_for(parent)
        .modal(true)
        .default_width(SMART_PLAYLIST_EDITOR_WIDTH + WINDOW_SHADOW_MARGIN * 2)
        .default_height(SMART_PLAYLIST_EDITOR_HEIGHT + WINDOW_SHADOW_MARGIN * 2)
        .resizable(false)
        .build();
    window.add_css_class("app-window");

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("preferences-frame");
    frame.set_hexpand(true);
    frame.set_vexpand(true);
    frame.set_margin_top(WINDOW_SHADOW_MARGIN);
    frame.set_margin_end(WINDOW_SHADOW_MARGIN);
    frame.set_margin_bottom(WINDOW_SHADOW_MARGIN);
    frame.set_margin_start(WINDOW_SHADOW_MARGIN);
    frame.set_size_request(SMART_PLAYLIST_EDITOR_WIDTH, SMART_PLAYLIST_EDITOR_HEIGHT);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("preferences-panel");
    panel.set_hexpand(true);
    panel.set_vexpand(true);
    panel.set_overflow(gtk::Overflow::Hidden);

    let close_row = close_row_for(&window);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(16);
    content.set_margin_end(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);

    let match_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let match_prefix = gtk::Label::new(Some("Match"));
    let match_combo = gtk::DropDown::new(
        Some(gtk::StringList::new(
            &MATCH_KINDS
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>(),
        )),
        gtk::Expression::NONE,
    );
    match_combo.set_selected(0);
    let match_suffix = gtk::Label::new(Some("of the following rules:"));
    match_row.append(&match_prefix);
    match_row.append(&match_combo);
    match_row.append(&match_suffix);
    content.append(&match_row);

    let rules_container = gtk::Box::new(gtk::Orientation::Vertical, 6);
    rules_container.add_css_class("smart-playlist-rules");
    content.append(&rules_container);

    let rule_rows: Rc<RefCell<Vec<RuleRow>>> = Rc::new(RefCell::new(Vec::new()));
    append_rule_row(&rules_container, &rule_rows);

    let limit_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    limit_row.add_css_class("smart-playlist-limit-row");
    let limit_check = gtk::CheckButton::with_label("Limit to");
    let limit_count = gtk::SpinButton::with_range(1.0, 10_000.0, 1.0);
    limit_count.set_value(25.0);
    limit_count.set_digits(0);
    limit_count.set_sensitive(false);
    let limit_unit = gtk::Label::new(Some("songs, selected by"));
    let limit_selection_combo = gtk::DropDown::new(
        Some(gtk::StringList::new(
            &LIMIT_SELECTIONS
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>(),
        )),
        gtk::Expression::NONE,
    );
    limit_selection_combo.set_selected(0);
    limit_selection_combo.set_sensitive(false);

    let limit_count_for_toggle = limit_count.clone();
    let limit_selection_for_toggle = limit_selection_combo.clone();
    limit_check.connect_toggled(move |check| {
        let active = check.is_active();
        limit_count_for_toggle.set_sensitive(active);
        limit_selection_for_toggle.set_sensitive(active);
    });

    limit_row.append(&limit_check);
    limit_row.append(&limit_count);
    limit_row.append(&limit_unit);
    limit_row.append(&limit_selection_combo);
    content.append(&limit_row);

    let error_label = gtk::Label::new(None);
    error_label.add_css_class("smart-playlist-error");
    error_label.set_xalign(0.0);
    error_label.set_wrap(true);
    error_label.set_visible(false);
    content.append(&error_label);

    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    content.append(&spacer);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);

    let cancel_button = gtk::Button::with_label("Cancel");
    let ok_button = gtk::Button::with_label("OK");
    ok_button.add_css_class("suggested-action");

    let window_for_cancel = window.clone();
    cancel_button.connect_clicked(move |_| {
        window_for_cancel.close();
    });

    let window_for_ok = window.clone();
    let command_controller_for_ok = command_controller.clone();
    let rule_rows_for_ok = rule_rows.clone();
    let match_combo_for_ok = match_combo.clone();
    let limit_check_for_ok = limit_check.clone();
    let limit_count_for_ok = limit_count.clone();
    let limit_selection_for_ok = limit_selection_combo.clone();
    let on_created_for_ok = on_created.clone();
    let error_label_for_ok = error_label.clone();
    let default_name_for_ok = default_name.clone();
    ok_button.connect_clicked(move |_| {
        let extraction = extract_rule_set(
            &rule_rows_for_ok.borrow(),
            &match_combo_for_ok,
            &limit_check_for_ok,
            &limit_count_for_ok,
            &limit_selection_for_ok,
        );
        match extraction {
            Ok(rule_set) => {
                let dispatched = command_controller_for_ok.dispatch_succeeded(
                    ApplicationCommand::CreateSmartPlaylist {
                        name: default_name_for_ok.clone(),
                        parent_folder_id: None,
                        rules: rule_set,
                    },
                );
                if dispatched {
                    on_created_for_ok();
                    window_for_ok.close();
                }
            }
            Err(error) => {
                error_label_for_ok.set_text(&error.message());
                error_label_for_ok.set_visible(true);
            }
        }
    });

    buttons.append(&cancel_button);
    buttons.append(&ok_button);
    content.append(&buttons);

    panel.append(&close_row);
    panel.append(&content);
    frame.append(&panel);
    window.set_child(Some(&frame));

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
    ok_button.grab_focus();
}

fn append_rule_row(rules_container: &gtk::Box, rule_rows: &Rc<RefCell<Vec<RuleRow>>>) {
    let row = RuleRow::new();
    let row_widget = row.container.clone();
    let row_buttons = rule_row_buttons();
    row_widget.append(&row_buttons.container);

    rules_container.append(&row_widget);
    rule_rows.borrow_mut().push(row);

    connect_row_buttons(
        row_buttons,
        row_widget,
        rule_rows.clone(),
        rules_container.clone(),
    );
}

fn close_row_for(window: &gtk::Window) -> gtk::Box {
    let close_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    close_row.set_margin_top(8);
    close_row.set_margin_end(8);
    close_row.set_margin_start(8);

    let close_icon = gtk::Image::from_icon_name("window-close-symbolic");
    close_icon.set_pixel_size(14);

    let close_button = gtk::Button::new();
    close_button.add_css_class("flat");
    close_button.add_css_class("preference-close-button");
    close_button.set_child(Some(&close_icon));
    close_button.set_tooltip_text(Some("Close"));
    close_button.set_halign(gtk::Align::End);
    close_button.set_valign(gtk::Align::Center);
    close_button.set_hexpand(true);

    let window_for_close = window.clone();
    close_button.connect_clicked(move |_| {
        window_for_close.close();
    });
    close_row.append(&close_button);
    close_row
}

#[derive(Clone)]
struct RuleRowButtons {
    container: gtk::Box,
    remove_button: gtk::Button,
    add_button: gtk::Button,
}

fn rule_row_buttons() -> RuleRowButtons {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);

    let remove_button = gtk::Button::from_icon_name("list-remove-symbolic");
    remove_button.add_css_class("flat");
    remove_button.set_tooltip_text(Some("Remove rule"));

    let add_button = gtk::Button::from_icon_name("list-add-symbolic");
    add_button.add_css_class("flat");
    add_button.set_tooltip_text(Some("Add rule"));

    container.append(&remove_button);
    container.append(&add_button);

    RuleRowButtons {
        container,
        remove_button,
        add_button,
    }
}

fn connect_row_buttons(
    buttons: RuleRowButtons,
    row_widget: gtk::Box,
    rule_rows: Rc<RefCell<Vec<RuleRow>>>,
    rules_container: gtk::Box,
) {
    let rule_rows_for_remove = rule_rows.clone();
    let rules_container_for_remove = rules_container.clone();
    let row_widget_for_remove = row_widget.clone();
    buttons.remove_button.connect_clicked(move |_| {
        let mut rows = rule_rows_for_remove.borrow_mut();
        if rows.len() <= 1 {
            return;
        }
        if let Some(index) = rows
            .iter()
            .position(|row| row.container == row_widget_for_remove)
        {
            rows.remove(index);
            rules_container_for_remove.remove(&row_widget_for_remove);
        }
    });

    let rule_rows_for_add = rule_rows.clone();
    let rules_container_for_add = rules_container.clone();
    buttons.add_button.connect_clicked(move |_| {
        append_rule_row(&rules_container_for_add, &rule_rows_for_add);
    });
}

fn extract_rule_set(
    rule_rows: &[RuleRow],
    match_combo: &gtk::DropDown,
    limit_check: &gtk::CheckButton,
    limit_count: &gtk::SpinButton,
    limit_selection_combo: &gtk::DropDown,
) -> Result<SmartPlaylistRuleSet, RuleError> {
    let rules: Vec<_> = rule_rows
        .iter()
        .map(RuleRow::extract)
        .collect::<Result<_, _>>()?;

    let limit = if limit_check.is_active() {
        let count_value = limit_count.value_as_int().max(1) as u32;
        let count = NonZeroU32::new(count_value).expect("count clamped to >= 1");
        Some(SmartPlaylistLimit {
            count,
            selection: limit_selection_from_combo(limit_selection_combo),
        })
    } else {
        None
    };

    Ok(SmartPlaylistRuleSet {
        match_kind: match_kind_from_combo(match_combo),
        rules,
        limit,
    })
}

fn match_kind_from_combo(combo: &gtk::DropDown) -> SmartPlaylistMatchKind {
    let index = combo.selected() as usize;
    MATCH_KINDS
        .get(index)
        .map(|(kind, _)| *kind)
        .unwrap_or(SmartPlaylistMatchKind::All)
}

fn limit_selection_from_combo(combo: &gtk::DropDown) -> SmartPlaylistLimitSelection {
    let index = combo.selected() as usize;
    LIMIT_SELECTIONS
        .get(index)
        .map(|(selection, _)| *selection)
        .unwrap_or(SmartPlaylistLimitSelection::Random)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_operator_table_matches_text_operator_enum_count() {
        assert_eq!(TEXT_OPERATORS.len(), 6);
    }

    #[test]
    fn number_operator_table_matches_number_operator_enum_count() {
        assert_eq!(NUMBER_OPERATORS.len(), 6);
    }

    #[test]
    fn limit_selection_table_excludes_no_selection_methods() {
        for selection in [
            SmartPlaylistLimitSelection::Random,
            SmartPlaylistLimitSelection::AlbumAscending,
            SmartPlaylistLimitSelection::ArtistAscending,
            SmartPlaylistLimitSelection::GenreAscending,
            SmartPlaylistLimitSelection::TitleAscending,
            SmartPlaylistLimitSelection::HighestRating,
            SmartPlaylistLimitSelection::LowestRating,
            SmartPlaylistLimitSelection::MostRecentlyPlayed,
            SmartPlaylistLimitSelection::LeastRecentlyPlayed,
            SmartPlaylistLimitSelection::MostOftenPlayed,
            SmartPlaylistLimitSelection::LeastOftenPlayed,
            SmartPlaylistLimitSelection::MostRecentlyAdded,
            SmartPlaylistLimitSelection::LeastRecentlyAdded,
        ] {
            assert!(
                LIMIT_SELECTIONS
                    .iter()
                    .any(|(item, _)| *item == selection),
                "expected {selection:?} to be available in the editor"
            );
        }
    }

    #[test]
    fn empty_text_value_yields_friendly_message() {
        let error = RuleError::EmptyTextValue;
        assert_eq!(error.message(), "A text rule needs a value.");
    }

    #[test]
    fn non_numeric_value_message_quotes_the_offending_input() {
        let error = RuleError::NonNumericValue {
            offending: "abc".to_owned(),
        };
        assert_eq!(error.message(), "\"abc\" is not a valid number.");
    }

    fn editor_field_index_for(target: EditorField) -> usize {
        EDITOR_FIELDS
            .iter()
            .position(|(field, _)| *field == target)
            .expect("editor field is registered")
    }

    fn text_operator_index_for(target: SmartPlaylistTextOperator) -> usize {
        TEXT_OPERATORS
            .iter()
            .position(|(operator, _)| *operator == target)
            .expect("text operator is registered")
    }

    fn number_operator_index_for(target: SmartPlaylistNumberOperator) -> usize {
        NUMBER_OPERATORS
            .iter()
            .position(|(operator, _)| *operator == target)
            .expect("number operator is registered")
    }

    #[test]
    fn extract_text_rule_trims_whitespace_and_returns_text_variant() {
        let rule = extract_rule(
            editor_field_index_for(EditorField::Text(SmartPlaylistTextField::Genre)),
            text_operator_index_for(SmartPlaylistTextOperator::Contains),
            "  Trip-Hop  ",
        )
        .expect("text rule extracts");

        assert_eq!(
            rule,
            SmartPlaylistRule::Text {
                field: SmartPlaylistTextField::Genre,
                operator: SmartPlaylistTextOperator::Contains,
                value: "Trip-Hop".to_owned(),
            }
        );
    }

    #[test]
    fn extract_text_rule_rejects_empty_value() {
        let result = extract_rule(
            editor_field_index_for(EditorField::Text(SmartPlaylistTextField::Title)),
            text_operator_index_for(SmartPlaylistTextOperator::Is),
            "   ",
        );

        assert_eq!(result, Err(RuleError::EmptyTextValue));
    }

    #[test]
    fn extract_number_rule_parses_signed_integer_value() {
        let rule = extract_rule(
            editor_field_index_for(EditorField::Number(SmartPlaylistNumberField::PlayCount)),
            number_operator_index_for(SmartPlaylistNumberOperator::GreaterThanOrEqual),
            "12",
        )
        .expect("number rule extracts");

        assert_eq!(
            rule,
            SmartPlaylistRule::Number {
                field: SmartPlaylistNumberField::PlayCount,
                operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
                value: 12,
            }
        );
    }

    #[test]
    fn extract_number_rule_rejects_non_numeric_value() {
        let result = extract_rule(
            editor_field_index_for(EditorField::Number(SmartPlaylistNumberField::Year)),
            number_operator_index_for(SmartPlaylistNumberOperator::Equal),
            "nineteen",
        );

        assert_eq!(
            result,
            Err(RuleError::NonNumericValue {
                offending: "nineteen".to_owned(),
            })
        );
    }
}
