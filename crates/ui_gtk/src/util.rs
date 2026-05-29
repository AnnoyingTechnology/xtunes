// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Small leaf helpers shared across the GTK view models.

use gtk::prelude::*;

/// Unicode glyph for an unfilled rating star.
const EMPTY_STAR: &str = "☆";
/// Unicode glyph for a filled rating star.
const FILLED_STAR: &str = "★";

/// Returns the trimmed text of an optional string field, or `None` when the
/// value is absent or blank once trimmed. Used by the view models to fall back
/// to placeholder text for missing metadata.
pub(crate) fn non_empty_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Renders one rating-star button at position `star` against the current
/// `rating`: sets the filled/empty glyph and toggles the `rating-star-filled`
/// / `rating-star-empty` CSS classes. Shared by the track table cells and the
/// track-info detail page so both style identically.
pub(crate) fn sync_rating_button(button: &gtk::Button, star: u8, rating: u8) {
    button.remove_css_class("rating-star-filled");
    button.remove_css_class("rating-star-empty");
    if star <= rating {
        button.set_label(FILLED_STAR);
        button.add_css_class("rating-star-filled");
    } else {
        button.set_label(EMPTY_STAR);
        button.add_css_class("rating-star-empty");
    }
}
