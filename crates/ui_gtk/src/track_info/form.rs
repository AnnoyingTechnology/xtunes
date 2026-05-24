// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::prelude::*;

pub(super) fn attach_field(
    grid: &gtk::Grid,
    row: i32,
    label_text: &str,
    field: &impl IsA<gtk::Widget>,
) {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(1.0);
    label.set_valign(gtk::Align::Center);
    label.add_css_class("track-info-field-label");
    grid.attach(&label, 0, row, 1, 1);
    grid.attach(field, 1, row, 3, 1);
}

pub(super) fn attach_paired_field(
    grid: &gtk::Grid,
    row: i32,
    label_text: &str,
    first: &gtk::Entry,
    second: &gtk::Entry,
) {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(1.0);
    label.set_valign(gtk::Align::Center);
    label.add_css_class("track-info-field-label");
    grid.attach(&label, 0, row, 1, 1);
    grid.attach(first, 1, row, 1, 1);

    let separator = gtk::Label::new(Some("of"));
    separator.add_css_class("dim-label");
    grid.attach(&separator, 2, row, 1, 1);
    grid.attach(second, 3, row, 1, 1);
}

pub(super) fn attach_readonly_field(grid: &gtk::Grid, row: i32, label_text: &str, value: &str) {
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(1.0);
    label.set_valign(gtk::Align::Center);
    label.add_css_class("track-info-field-label");
    grid.attach(&label, 0, row, 1, 1);

    // Single-line ellipsize keeps the row at a known height regardless of
    // value length. A wrapping label with no line cap reports
    // min_height = full_text_wrapped_at_minimum_width when GTK measures it
    // narrow, which makes the whole dialog claim thousands of pixels of
    // minimum height. Ellipsize-Middle is the standard idiom for paths.
    let value_label = gtk::Label::new(Some(value));
    value_label.set_xalign(0.0);
    value_label.set_valign(gtk::Align::Center);
    value_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    value_label.set_selectable(true);
    value_label.set_hexpand(true);
    value_label.set_tooltip_text(Some(value));
    grid.attach(&value_label, 1, row, 1, 1);
}

pub(super) fn text_entry(initial: Option<&str>) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    if let Some(text) = initial {
        entry.set_text(text);
    }
    entry
}

pub(super) fn number_entry<T: ToString>(initial: Option<T>, width_chars: i32) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_width_chars(width_chars);
    entry.set_max_width_chars(width_chars);
    entry.set_hexpand(false);
    entry.set_halign(gtk::Align::Start);
    if let Some(value) = initial {
        entry.set_text(&value.to_string());
    }
    entry
}
