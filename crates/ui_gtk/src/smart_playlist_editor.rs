// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::{Cell, RefCell},
    num::NonZeroU32,
    rc::Rc,
    time::{Duration, SystemTime},
};

use gtk::prelude::*;
use gtk::{gdk, glib};

use xtunes_app_runtime::{
    ApplicationCommand, Rating, SmartPlaylistDateField, SmartPlaylistLimit,
    SmartPlaylistLimitSelection, SmartPlaylistMatchKind, SmartPlaylistNumberField,
    SmartPlaylistNumberOperator, SmartPlaylistRule, SmartPlaylistRuleSet, SmartPlaylistTextField,
    SmartPlaylistTextOperator,
};

use super::{
    SMART_PLAYLIST_EDITOR_HEIGHT, SMART_PLAYLIST_EDITOR_WIDTH, WINDOW_SHADOW_MARGIN,
    command_controller::SharedCommandController,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorField {
    Text(SmartPlaylistTextField),
    Number(SmartPlaylistNumberField),
    Rating,
    Date(SmartPlaylistDateField),
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
    (
        EditorField::Text(SmartPlaylistTextField::FileName),
        "File Name",
    ),
    (EditorField::Rating, "Rating"),
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
    (
        EditorField::Date(SmartPlaylistDateField::DateAdded),
        "Date Added",
    ),
    (
        EditorField::Date(SmartPlaylistDateField::LastPlayed),
        "Last Played",
    ),
    (
        EditorField::Date(SmartPlaylistDateField::LastSkipped),
        "Last Skipped",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorOperator {
    TextContains,
    TextDoesNotContain,
    TextIs,
    TextIsNot,
    TextStartsWith,
    TextEndsWith,
    TextIsEmpty,
    TextIsPresent,
    NumberEqual,
    NumberNotEqual,
    NumberGreaterThan,
    NumberGreaterThanOrEqual,
    NumberLessThan,
    NumberLessThanOrEqual,
    DateBefore,
    DateAfter,
    DateInLast,
    DateNotInLast,
    DateIsEmpty,
    DateIsPresent,
}

impl EditorOperator {
    fn label(self) -> &'static str {
        match self {
            Self::TextContains => "contains",
            Self::TextDoesNotContain => "does not contain",
            Self::TextIs => "is",
            Self::TextIsNot => "is not",
            Self::TextStartsWith => "starts with",
            Self::TextEndsWith => "ends with",
            Self::TextIsEmpty => "is empty",
            Self::TextIsPresent => "is present",
            Self::NumberEqual => "is",
            Self::NumberNotEqual => "is not",
            Self::NumberGreaterThan => "is greater than",
            Self::NumberGreaterThanOrEqual => "is greater than or equal to",
            Self::NumberLessThan => "is less than",
            Self::NumberLessThanOrEqual => "is less than or equal to",
            Self::DateBefore => "is before",
            Self::DateAfter => "is after",
            Self::DateInLast => "is in the last",
            Self::DateNotInLast => "is not in the last",
            Self::DateIsEmpty => "is empty",
            Self::DateIsPresent => "is present",
        }
    }

    fn value_kind(self) -> ValueKind {
        match self {
            Self::TextContains
            | Self::TextDoesNotContain
            | Self::TextIs
            | Self::TextIsNot
            | Self::TextStartsWith
            | Self::TextEndsWith => ValueKind::Text,
            Self::TextIsEmpty | Self::TextIsPresent => ValueKind::None,
            Self::NumberEqual
            | Self::NumberNotEqual
            | Self::NumberGreaterThan
            | Self::NumberGreaterThanOrEqual
            | Self::NumberLessThan
            | Self::NumberLessThanOrEqual => ValueKind::Number,
            Self::DateBefore | Self::DateAfter => ValueKind::Date,
            Self::DateInLast | Self::DateNotInLast => ValueKind::Days,
            Self::DateIsEmpty | Self::DateIsPresent => ValueKind::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValueKind {
    Text,
    Number,
    Rating,
    Date,
    Days,
    None,
}

const TEXT_OPERATORS: &[EditorOperator] = &[
    EditorOperator::TextContains,
    EditorOperator::TextDoesNotContain,
    EditorOperator::TextIs,
    EditorOperator::TextIsNot,
    EditorOperator::TextStartsWith,
    EditorOperator::TextEndsWith,
    EditorOperator::TextIsEmpty,
    EditorOperator::TextIsPresent,
];

const NUMBER_OPERATORS: &[EditorOperator] = &[
    EditorOperator::NumberEqual,
    EditorOperator::NumberNotEqual,
    EditorOperator::NumberGreaterThan,
    EditorOperator::NumberGreaterThanOrEqual,
    EditorOperator::NumberLessThan,
    EditorOperator::NumberLessThanOrEqual,
];

const DATE_OPERATORS: &[EditorOperator] = &[
    EditorOperator::DateBefore,
    EditorOperator::DateAfter,
    EditorOperator::DateInLast,
    EditorOperator::DateNotInLast,
    EditorOperator::DateIsEmpty,
    EditorOperator::DateIsPresent,
];

fn operators_for_field(field: EditorField) -> &'static [EditorOperator] {
    match field {
        EditorField::Text(_) => TEXT_OPERATORS,
        EditorField::Number(_) | EditorField::Rating => NUMBER_OPERATORS,
        EditorField::Date(_) => DATE_OPERATORS,
    }
}

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
enum ValueWidget {
    Text(gtk::Entry),
    Number(gtk::SpinButton),
    Rating(gtk::SpinButton),
    Date(gtk::Entry),
    Days(gtk::SpinButton),
    None,
}

impl ValueWidget {
    fn root(&self) -> Option<gtk::Widget> {
        match self {
            Self::Text(entry) => Some(entry.clone().upcast()),
            Self::Number(spin) => Some(spin.clone().upcast()),
            Self::Rating(spin) => Some(spin.clone().upcast()),
            Self::Date(entry) => Some(entry.clone().upcast()),
            Self::Days(spin) => Some(spin.clone().upcast()),
            Self::None => None,
        }
    }

    fn build(kind: ValueKind) -> Self {
        match kind {
            ValueKind::Text => {
                let entry = gtk::Entry::new();
                entry.set_hexpand(true);
                entry.set_placeholder_text(Some("value"));
                Self::Text(entry)
            }
            ValueKind::Number => {
                let spin = gtk::SpinButton::with_range(0.0, 9_999_999.0, 1.0);
                spin.set_value(0.0);
                spin.set_digits(0);
                Self::Number(spin)
            }
            ValueKind::Rating => {
                let spin = gtk::SpinButton::with_range(0.0, 5.0, 1.0);
                spin.set_value(0.0);
                spin.set_digits(0);
                Self::Rating(spin)
            }
            ValueKind::Date => {
                let entry = gtk::Entry::new();
                entry.set_placeholder_text(Some("YYYY-MM-DD"));
                entry.set_max_length(10);
                entry.set_width_chars(11);
                Self::Date(entry)
            }
            ValueKind::Days => {
                let spin = gtk::SpinButton::with_range(1.0, 9_999.0, 1.0);
                spin.set_value(7.0);
                spin.set_digits(0);
                Self::Days(spin)
            }
            ValueKind::None => Self::None,
        }
    }
}

#[derive(Clone)]
struct RuleRow {
    container: gtk::Box,
    current_field: Rc<Cell<EditorField>>,
    current_operator: Rc<Cell<EditorOperator>>,
    current_value: Rc<RefCell<ValueWidget>>,
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

        let initial_field = EDITOR_FIELDS[0].0;
        let initial_operator = operators_for_field(initial_field)[0];

        let operator_combo = gtk::DropDown::new(
            Some(operator_model_for_field(initial_field)),
            gtk::Expression::NONE,
        );
        operator_combo.set_selected(0);

        let value_container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        value_container.set_hexpand(true);

        let days_suffix = gtk::Label::new(Some("days"));
        days_suffix.set_visible(false);

        let current_field = Rc::new(Cell::new(initial_field));
        let current_operator = Rc::new(Cell::new(initial_operator));
        let current_value = Rc::new(RefCell::new(ValueWidget::None));

        install_value_widget(
            &value_container,
            &days_suffix,
            &current_value,
            initial_field,
            initial_operator,
        );

        container.append(&field_combo);
        container.append(&operator_combo);
        container.append(&value_container);
        container.append(&days_suffix);

        let operator_combo_for_field = operator_combo.clone();
        let current_field_for_field = current_field.clone();
        let current_operator_for_field = current_operator.clone();
        let current_value_for_field = current_value.clone();
        let value_container_for_field = value_container.clone();
        let days_suffix_for_field = days_suffix.clone();
        field_combo.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected() as usize;
            let Some((field, _)) = EDITOR_FIELDS.get(index).copied() else {
                return;
            };
            current_field_for_field.set(field);
            operator_combo_for_field.set_model(Some(&operator_model_for_field(field)));
            operator_combo_for_field.set_selected(0);
            let operator = operators_for_field(field)[0];
            current_operator_for_field.set(operator);
            install_value_widget(
                &value_container_for_field,
                &days_suffix_for_field,
                &current_value_for_field,
                field,
                operator,
            );
        });

        let current_field_for_op = current_field.clone();
        let current_operator_for_op = current_operator.clone();
        let current_value_for_op = current_value.clone();
        let value_container_for_op = value_container.clone();
        let days_suffix_for_op = days_suffix.clone();
        operator_combo.connect_selected_notify(move |dropdown| {
            let field = current_field_for_op.get();
            let operators = operators_for_field(field);
            let index = dropdown.selected() as usize;
            let Some(operator) = operators.get(index).copied() else {
                return;
            };
            current_operator_for_op.set(operator);
            install_value_widget(
                &value_container_for_op,
                &days_suffix_for_op,
                &current_value_for_op,
                field,
                operator,
            );
        });

        Self {
            container,
            current_field,
            current_operator,
            current_value,
        }
    }

    fn extract(&self) -> Result<SmartPlaylistRule, RuleError> {
        let input = value_input_from_widget(&self.current_value.borrow());
        extract_rule(self.current_field.get(), self.current_operator.get(), &input)
    }
}

fn value_input_from_widget(value: &ValueWidget) -> ValueInput {
    match value {
        ValueWidget::Text(entry) => ValueInput::Text(entry.text().to_string()),
        ValueWidget::Number(spin) => ValueInput::Number(spin.value_as_int() as i64),
        ValueWidget::Rating(spin) => ValueInput::Rating(spin.value_as_int().max(0) as u32),
        ValueWidget::Date(entry) => ValueInput::Date(entry.text().to_string()),
        ValueWidget::Days(spin) => ValueInput::Days(spin.value_as_int().max(0) as u32),
        ValueWidget::None => ValueInput::None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ValueInput {
    Text(String),
    Number(i64),
    Rating(u32),
    Date(String),
    Days(u32),
    None,
}

fn install_value_widget(
    container: &gtk::Box,
    days_suffix: &gtk::Label,
    current_value: &Rc<RefCell<ValueWidget>>,
    field: EditorField,
    operator: EditorOperator,
) {
    let kind = effective_value_kind(field, operator);
    let new_widget = ValueWidget::build(kind);

    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    if let Some(root) = new_widget.root() {
        container.append(&root);
    }
    days_suffix.set_visible(matches!(new_widget, ValueWidget::Days(_)));
    current_value.replace(new_widget);
}

fn effective_value_kind(field: EditorField, operator: EditorOperator) -> ValueKind {
    match (field, operator.value_kind()) {
        (EditorField::Rating, ValueKind::Number) => ValueKind::Rating,
        (_, kind) => kind,
    }
}

fn operator_model_for_field(field: EditorField) -> gtk::StringList {
    let labels: Vec<&str> = operators_for_field(field)
        .iter()
        .map(|op| op.label())
        .collect();
    gtk::StringList::new(&labels)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RuleError {
    FieldOperatorMismatch,
    EmptyTextValue,
    OutOfRangeRating { offending: String },
    InvalidDate { offending: String },
    InvalidDays,
}

impl RuleError {
    fn message(&self) -> String {
        match self {
            Self::FieldOperatorMismatch => {
                "This combination of field and operator is not supported.".to_owned()
            }
            Self::EmptyTextValue => "A text rule needs a value.".to_owned(),
            Self::OutOfRangeRating { offending } => {
                format!("\"{offending}\" is not a valid rating (0 to 5).")
            }
            Self::InvalidDate { offending } => {
                format!("\"{offending}\" is not a valid date (use YYYY-MM-DD).")
            }
            Self::InvalidDays => "The number of days must be at least 1.".to_owned(),
        }
    }
}

fn extract_rule(
    field: EditorField,
    operator: EditorOperator,
    value: &ValueInput,
) -> Result<SmartPlaylistRule, RuleError> {
    match (field, operator) {
        (EditorField::Text(text_field), op) if matches!(op.value_kind(), ValueKind::Text) => {
            let raw = read_text(value)?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(RuleError::EmptyTextValue);
            }
            Ok(SmartPlaylistRule::Text {
                field: text_field,
                operator: text_operator(op).ok_or(RuleError::FieldOperatorMismatch)?,
                value: trimmed.to_owned(),
            })
        }
        (EditorField::Text(text_field), EditorOperator::TextIsEmpty) => {
            Ok(SmartPlaylistRule::TextIsEmpty { field: text_field })
        }
        (EditorField::Text(text_field), EditorOperator::TextIsPresent) => {
            Ok(SmartPlaylistRule::TextIsPresent { field: text_field })
        }
        (EditorField::Number(number_field), op)
            if matches!(op.value_kind(), ValueKind::Number) =>
        {
            let parsed = read_number(value)?;
            Ok(SmartPlaylistRule::Number {
                field: number_field,
                operator: number_operator(op).ok_or(RuleError::FieldOperatorMismatch)?,
                value: parsed,
            })
        }
        (EditorField::Rating, op) if matches!(op.value_kind(), ValueKind::Number) => {
            let parsed = read_rating(value)?;
            Ok(SmartPlaylistRule::Rating {
                operator: number_operator(op).ok_or(RuleError::FieldOperatorMismatch)?,
                value: parsed,
            })
        }
        (EditorField::Date(date_field), EditorOperator::DateBefore) => {
            Ok(SmartPlaylistRule::DateBefore {
                field: date_field,
                date: read_date(value)?,
            })
        }
        (EditorField::Date(date_field), EditorOperator::DateAfter) => {
            Ok(SmartPlaylistRule::DateAfter {
                field: date_field,
                date: read_date(value)?,
            })
        }
        (EditorField::Date(date_field), EditorOperator::DateInLast) => {
            Ok(SmartPlaylistRule::DateInLast {
                field: date_field,
                days: read_days(value)?,
            })
        }
        (EditorField::Date(date_field), EditorOperator::DateNotInLast) => {
            Ok(SmartPlaylistRule::DateNotInLast {
                field: date_field,
                days: read_days(value)?,
            })
        }
        (EditorField::Date(date_field), EditorOperator::DateIsEmpty) => {
            Ok(SmartPlaylistRule::DateIsEmpty { field: date_field })
        }
        (EditorField::Date(date_field), EditorOperator::DateIsPresent) => {
            Ok(SmartPlaylistRule::DateIsPresent { field: date_field })
        }
        _ => Err(RuleError::FieldOperatorMismatch),
    }
}

fn read_text(value: &ValueInput) -> Result<String, RuleError> {
    match value {
        ValueInput::Text(text) => Ok(text.clone()),
        _ => Err(RuleError::FieldOperatorMismatch),
    }
}

fn read_number(value: &ValueInput) -> Result<i64, RuleError> {
    match value {
        ValueInput::Number(number) => Ok(*number),
        _ => Err(RuleError::FieldOperatorMismatch),
    }
}

fn read_rating(value: &ValueInput) -> Result<Rating, RuleError> {
    match value {
        ValueInput::Rating(raw) => {
            let stars = u8::try_from(*raw).map_err(|_| RuleError::OutOfRangeRating {
                offending: raw.to_string(),
            })?;
            Rating::new(stars).ok_or(RuleError::OutOfRangeRating {
                offending: stars.to_string(),
            })
        }
        _ => Err(RuleError::FieldOperatorMismatch),
    }
}

fn read_date(value: &ValueInput) -> Result<SystemTime, RuleError> {
    match value {
        ValueInput::Date(text) => parse_iso_date(text.trim()).ok_or(RuleError::InvalidDate {
            offending: text.clone(),
        }),
        _ => Err(RuleError::FieldOperatorMismatch),
    }
}

fn read_days(value: &ValueInput) -> Result<NonZeroU32, RuleError> {
    match value {
        ValueInput::Days(raw) => NonZeroU32::new(*raw).ok_or(RuleError::InvalidDays),
        _ => Err(RuleError::FieldOperatorMismatch),
    }
}

fn text_operator(operator: EditorOperator) -> Option<SmartPlaylistTextOperator> {
    Some(match operator {
        EditorOperator::TextContains => SmartPlaylistTextOperator::Contains,
        EditorOperator::TextDoesNotContain => SmartPlaylistTextOperator::DoesNotContain,
        EditorOperator::TextIs => SmartPlaylistTextOperator::Is,
        EditorOperator::TextIsNot => SmartPlaylistTextOperator::IsNot,
        EditorOperator::TextStartsWith => SmartPlaylistTextOperator::StartsWith,
        EditorOperator::TextEndsWith => SmartPlaylistTextOperator::EndsWith,
        _ => return None,
    })
}

fn number_operator(operator: EditorOperator) -> Option<SmartPlaylistNumberOperator> {
    Some(match operator {
        EditorOperator::NumberEqual => SmartPlaylistNumberOperator::Equal,
        EditorOperator::NumberNotEqual => SmartPlaylistNumberOperator::NotEqual,
        EditorOperator::NumberGreaterThan => SmartPlaylistNumberOperator::GreaterThan,
        EditorOperator::NumberGreaterThanOrEqual => SmartPlaylistNumberOperator::GreaterThanOrEqual,
        EditorOperator::NumberLessThan => SmartPlaylistNumberOperator::LessThan,
        EditorOperator::NumberLessThanOrEqual => SmartPlaylistNumberOperator::LessThanOrEqual,
        _ => return None,
    })
}

fn parse_iso_date(text: &str) -> Option<SystemTime> {
    let mut parts = text.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let max_day = days_in_month(year, month);
    if !(1..=max_day).contains(&day) {
        return None;
    }
    let days = days_since_unix_epoch(year, month, day)?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(days * 86_400))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_since_unix_epoch(year: i32, month: u32, day: u32) -> Option<u64> {
    if year < 1970 {
        return None;
    }
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    for m in 1..month {
        days += u64::from(days_in_month(year, m));
    }
    days += u64::from(day - 1);
    Some(days)
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
    fn parse_iso_date_accepts_padded_dates() {
        let date = parse_iso_date("2024-05-23").expect("valid date");
        let elapsed = date
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after epoch");
        let expected_days: u64 = (1970..2024)
            .map(|y| if is_leap_year(y) { 366 } else { 365 })
            .sum::<u64>()
            + 31
            + 29
            + 31
            + 30
            + 22;
        assert_eq!(elapsed.as_secs(), expected_days * 86_400);
    }

    #[test]
    fn parse_iso_date_rejects_invalid_dates() {
        assert!(parse_iso_date("2024-02-30").is_none());
        assert!(parse_iso_date("2024-13-01").is_none());
        assert!(parse_iso_date("not-a-date").is_none());
        assert!(parse_iso_date("2024-05").is_none());
        assert!(parse_iso_date("2024-05-23-extra").is_none());
        assert!(parse_iso_date("1969-12-31").is_none());
    }

    #[test]
    fn leap_year_rules_match_gregorian_calendar() {
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
    }

    #[test]
    fn extract_rule_text_is_empty_creates_text_is_empty_variant() {
        let rule = extract_rule(
            EditorField::Text(SmartPlaylistTextField::Genre),
            EditorOperator::TextIsEmpty,
            &ValueInput::None,
        )
        .expect("extracts");
        assert_eq!(
            rule,
            SmartPlaylistRule::TextIsEmpty {
                field: SmartPlaylistTextField::Genre,
            }
        );
    }

    #[test]
    fn extract_rule_rating_constructs_rating_variant_with_parsed_stars() {
        let rule = extract_rule(
            EditorField::Rating,
            EditorOperator::NumberGreaterThanOrEqual,
            &ValueInput::Rating(4),
        )
        .expect("extracts");
        assert_eq!(
            rule,
            SmartPlaylistRule::Rating {
                operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
                value: Rating::new(4).expect("valid"),
            }
        );
    }

    #[test]
    fn extract_rule_rating_rejects_out_of_range_value() {
        let result = extract_rule(
            EditorField::Rating,
            EditorOperator::NumberEqual,
            &ValueInput::Rating(9),
        );
        assert_eq!(
            result,
            Err(RuleError::OutOfRangeRating {
                offending: "9".to_owned(),
            })
        );
    }

    #[test]
    fn extract_rule_date_before_parses_iso_value() {
        let rule = extract_rule(
            EditorField::Date(SmartPlaylistDateField::DateAdded),
            EditorOperator::DateBefore,
            &ValueInput::Date("2024-05-23".to_owned()),
        )
        .expect("extracts");
        match rule {
            SmartPlaylistRule::DateBefore { field, date } => {
                assert_eq!(field, SmartPlaylistDateField::DateAdded);
                let expected = parse_iso_date("2024-05-23").expect("valid");
                assert_eq!(date, expected);
            }
            other => panic!("unexpected rule variant: {other:?}"),
        }
    }

    #[test]
    fn extract_rule_date_before_rejects_invalid_iso() {
        let result = extract_rule(
            EditorField::Date(SmartPlaylistDateField::DateAdded),
            EditorOperator::DateBefore,
            &ValueInput::Date("nope".to_owned()),
        );
        assert_eq!(
            result,
            Err(RuleError::InvalidDate {
                offending: "nope".to_owned(),
            })
        );
    }

    #[test]
    fn extract_rule_date_in_last_uses_days() {
        let rule = extract_rule(
            EditorField::Date(SmartPlaylistDateField::LastPlayed),
            EditorOperator::DateInLast,
            &ValueInput::Days(14),
        )
        .expect("extracts");
        match rule {
            SmartPlaylistRule::DateInLast { field, days } => {
                assert_eq!(field, SmartPlaylistDateField::LastPlayed);
                assert_eq!(days.get(), 14);
            }
            other => panic!("unexpected rule variant: {other:?}"),
        }
    }

    #[test]
    fn extract_rule_date_in_last_rejects_zero_days() {
        let result = extract_rule(
            EditorField::Date(SmartPlaylistDateField::LastPlayed),
            EditorOperator::DateInLast,
            &ValueInput::Days(0),
        );
        assert_eq!(result, Err(RuleError::InvalidDays));
    }

    #[test]
    fn effective_value_kind_uses_rating_for_rating_field() {
        assert_eq!(
            effective_value_kind(EditorField::Rating, EditorOperator::NumberEqual),
            ValueKind::Rating
        );
        assert_eq!(
            effective_value_kind(
                EditorField::Number(SmartPlaylistNumberField::Year),
                EditorOperator::NumberEqual,
            ),
            ValueKind::Number
        );
    }
}
