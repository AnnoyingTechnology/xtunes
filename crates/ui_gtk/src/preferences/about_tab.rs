// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::gio;
use gtk::prelude::*;

use super::{HELPER_MAX_WIDTH_CHARS, HELPER_MIN_WIDTH_CHARS};

/// Themed application icon, registered by `install_app_icon` in
/// `main_window.rs`. Matches the desktop/MPRIS identity so the About
/// pane shows the same artwork the user sees in their app launcher.
const APP_ICON_NAME: &str = "io.github.open_sustain.sustain";
/// Logical pixels. GTK picks the closest pre-built size from the
/// hicolor theme and scales it with the display scale factor, so this
/// looks crisp on HiDPI screens (256/512 sources are shipped under
/// `data/icons/hicolor`).
const APP_ICON_SIZE: i32 = 96;

const DOCUMENTATION_URL: &str =
    "https://github.com/open-sustain/sustain/blob/main/docs/features.md";
const ISSUES_URL: &str = "https://github.com/open-sustain/sustain/issues";

pub(super) fn build(parent_window: &gtk::Window) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_top(24);
    content.set_margin_end(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);

    let icon = gtk::Image::from_icon_name(APP_ICON_NAME);
    icon.set_pixel_size(APP_ICON_SIZE);
    icon.set_halign(gtk::Align::Center);
    icon.add_css_class("about-app-icon");
    content.append(&icon);

    let name = gtk::Label::new(Some("Sustain"));
    name.set_halign(gtk::Align::Center);
    name.add_css_class("about-app-name");
    content.append(&name);

    content.append(&centered_helper_label(
        "A music library and player for the Linux desktop.",
    ));

    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.set_halign(gtk::Align::Center);
    details.set_margin_top(8);
    details.append(&centered_helper_label(&format!(
        "Version {}",
        env!("CARGO_PKG_VERSION")
    )));
    details.append(&centered_helper_label("Licensed under GPL-3.0-or-later"));
    details.append(&centered_helper_label("© 2026 AnnoyingTechnology"));
    content.append(&details);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.set_halign(gtk::Align::Center);
    actions.set_margin_top(16);
    actions.set_homogeneous(true);

    let docs_button = gtk::Button::with_label("Documentation");
    docs_button.set_tooltip_text(Some(DOCUMENTATION_URL));
    let docs_parent = parent_window.clone();
    docs_button.connect_clicked(move |_| {
        launch_uri(&docs_parent, DOCUMENTATION_URL);
    });
    actions.append(&docs_button);

    let issues_button = gtk::Button::with_label("Report a problem");
    issues_button.set_tooltip_text(Some(ISSUES_URL));
    let issues_parent = parent_window.clone();
    issues_button.connect_clicked(move |_| {
        launch_uri(&issues_parent, ISSUES_URL);
    });
    actions.append(&issues_button);

    content.append(&actions);

    content.upcast()
}

/// Build a centered muted label whose natural width is bounded the same
/// way every other wrapping label in Preferences is bounded
/// (`wrap = true` + `width-chars` + `max-width-chars`, per commit
/// `12a91bb`). Short strings stay on one line; wrap only engages if the
/// text ever exceeds the 56-char ceiling.
fn centered_helper_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("preference-helper");
    label.set_halign(gtk::Align::Center);
    label.set_justify(gtk::Justification::Center);
    label.set_wrap(true);
    label.set_natural_wrap_mode(gtk::NaturalWrapMode::Word);
    label.set_width_chars(HELPER_MIN_WIDTH_CHARS);
    label.set_max_width_chars(HELPER_MAX_WIDTH_CHARS);
    label
}

/// Opens `url` in the user's default browser via `GtkUriLauncher`,
/// which routes through the desktop portal under Flatpak and the
/// `org.freedesktop.DBus`/`xdg-open` path otherwise. Failures are
/// logged: on a properly-configured Linux desktop, this only fails
/// when no default browser is registered, and there is no meaningful
/// fallback Sustain can offer.
fn launch_uri(parent: &gtk::Window, url: &'static str) {
    let launcher = gtk::UriLauncher::new(url);
    launcher.launch(Some(parent), None::<&gio::Cancellable>, move |result| {
        if let Err(error) = result {
            eprintln!("Sustain: failed to open {url} in the default browser ({error:?})");
        }
    });
}
