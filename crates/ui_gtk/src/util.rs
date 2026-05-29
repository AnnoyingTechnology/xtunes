// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Small leaf helpers shared across the GTK view models.

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
