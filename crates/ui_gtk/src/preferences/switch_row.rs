// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::prelude::*;

/// A label-on-the-left, switch-on-the-right row with a muted helper line
/// underneath. Shared layout for every capability toggle in the Preferences
/// window — keeping it in one place means the Library, Analysis, and Online
/// tabs cannot drift visually.
pub(super) struct SwitchRow {
    pub(super) container: gtk::Box,
    pub(super) switch: gtk::Switch,
}

pub(super) fn build_switch_row(label_text: &str, helper_text: &str, initial: bool) -> SwitchRow {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 4);
    container.add_css_class("preference-switch-row");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(0.0);
    label.set_hexpand(true);

    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    switch.set_active(initial);

    header.append(&label);
    header.append(&switch);

    let helper = gtk::Label::new(Some(helper_text));
    helper.add_css_class("preference-helper");
    helper.set_xalign(0.0);
    helper.set_wrap(true);

    container.append(&header);
    container.append(&helper);

    SwitchRow { container, switch }
}
