// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::time::{SystemTime, UNIX_EPOCH};

use gtk::glib;

/// Render a `SystemTime` as `dd/mm/YYYY HH:MM`.
///
/// Returns `None` when the timestamp is before the Unix epoch, which only
/// happens for pathological clocks; callers decide how to render that (em
/// dash in a details panel, empty cell in the table).
pub(crate) fn format_system_time_short(time: SystemTime) -> Option<String> {
    let since_epoch = time.duration_since(UNIX_EPOCH).ok()?;
    let seconds = i64::try_from(since_epoch.as_secs()).ok()?;
    let local = glib::DateTime::from_unix_local(seconds).ok()?;
    local
        .format("%d/%m/%Y %H:%M")
        .ok()
        .map(|text| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::format_system_time_short;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn format_system_time_short_renders_local_time_in_dmy_hm() {
        let time = UNIX_EPOCH + Duration::from_secs(1_577_882_700);
        let rendered = format_system_time_short(time).expect("formats local timestamp");

        assert_eq!(rendered.len(), "01/01/2020 12:45".len());
        assert_eq!(&rendered[2..3], "/");
        assert_eq!(&rendered[5..6], "/");
        assert_eq!(&rendered[10..11], " ");
        assert_eq!(&rendered[13..14], ":");
    }
}
