// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::time::{SystemTime, UNIX_EPOCH};

/// Render a `SystemTime` as `dd/mm/YYYY HH:MM`.
///
/// Returns `None` when the timestamp is before the Unix epoch, which only
/// happens for pathological clocks; callers decide how to render that (em
/// dash in a details panel, empty cell in the table).
pub(crate) fn format_system_time_short(time: SystemTime) -> Option<String> {
    let since_epoch = time.duration_since(UNIX_EPOCH).ok()?;
    Some(format_unix_seconds(since_epoch.as_secs()))
}

fn format_unix_seconds(unix_seconds: u64) -> String {
    let (year, month, day, hour, minute) = unix_seconds_to_ymdhm(unix_seconds);
    format!("{day:02}/{month:02}/{year:04} {hour:02}:{minute:02}")
}

fn unix_seconds_to_ymdhm(unix_seconds: u64) -> (i32, u32, u32, u32, u32) {
    // Howard Hinnant's civil-date algorithm, so we don't pull in a date crate
    // just to render a column.
    let total_minutes = unix_seconds / 60;
    let minute = (total_minutes % 60) as u32;
    let total_hours = total_minutes / 60;
    let hour = (total_hours % 24) as u32;
    let days_since_epoch = (total_hours / 24) as i64;

    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = (y + i64::from(month <= 2)) as i32;

    (year, month, day, hour, minute)
}

#[cfg(test)]
mod tests {
    use super::{format_system_time_short, unix_seconds_to_ymdhm};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn unix_seconds_to_ymdhm_matches_known_dates() {
        assert_eq!(unix_seconds_to_ymdhm(0), (1970, 1, 1, 0, 0));
        // 2000-01-01 00:00:00 UTC
        assert_eq!(unix_seconds_to_ymdhm(946_684_800), (2000, 1, 1, 0, 0));
        // 2020-01-01 12:45:00 UTC
        assert_eq!(unix_seconds_to_ymdhm(1_577_882_700), (2020, 1, 1, 12, 45));
    }

    #[test]
    fn format_system_time_short_renders_in_dmy_hm() {
        let time = UNIX_EPOCH + Duration::from_secs(1_577_882_700);
        assert_eq!(
            format_system_time_short(time).as_deref(),
            Some("01/01/2020 12:45")
        );
    }
}
