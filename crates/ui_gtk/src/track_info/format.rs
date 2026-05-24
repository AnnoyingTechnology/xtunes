// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) fn format_kind(path: Option<&Path>) -> String {
    let Some(path) = path else {
        return String::from("\u{2014}");
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("mp3") => "MPEG audio file".to_owned(),
        Some("flac") => "FLAC audio file".to_owned(),
        Some("ogg" | "oga") => "Ogg Vorbis audio file".to_owned(),
        Some("opus") => "Opus audio file".to_owned(),
        Some("m4a" | "m4b" | "mp4") => "MPEG-4 audio file".to_owned(),
        Some(other) => format!("{} audio file", other.to_ascii_uppercase()),
        None => "Audio file".to_owned(),
    }
}

pub(super) fn format_duration_label(duration: Option<Duration>) -> String {
    let Some(duration) = duration else {
        return String::from("\u{2014}");
    };
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub(super) fn format_size_label(size: Option<u64>) -> String {
    let Some(size) = size else {
        return String::from("\u{2014}");
    };
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let size_f = size as f64;
    if size_f >= GIB {
        format!("{:.2} GB", size_f / GIB)
    } else if size_f >= MIB {
        format!("{:.2} MB", size_f / MIB)
    } else if size_f >= KIB {
        format!("{:.2} KB", size_f / KIB)
    } else {
        format!("{size} B")
    }
}

pub(super) fn format_optional_unit<T: std::fmt::Display>(value: Option<T>, unit: &str) -> String {
    match value {
        Some(value) => format!("{value} {unit}"),
        None => String::from("\u{2014}"),
    }
}

pub(super) fn format_sample_rate(sample_rate_hz: Option<u32>) -> String {
    match sample_rate_hz {
        Some(hz) => {
            let khz = f64::from(hz) / 1000.0;
            if (khz.fract() - 0.0).abs() < f64::EPSILON {
                format!("{khz:.0} kHz")
            } else {
                format!("{khz:.1} kHz")
            }
        }
        None => String::from("\u{2014}"),
    }
}

pub(super) fn format_channels(channels: Option<u8>) -> String {
    match channels {
        Some(1) => "Mono".to_owned(),
        Some(2) => "Stereo".to_owned(),
        Some(count) => format!("{count} channels"),
        None => String::from("\u{2014}"),
    }
}

pub(super) fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return String::from("\u{2014}");
    };
    let Ok(since_epoch) = modified.duration_since(UNIX_EPOCH) else {
        return String::from("\u{2014}");
    };
    format_unix_seconds(since_epoch.as_secs())
}

fn format_unix_seconds(unix_seconds: u64) -> String {
    let (year, month, day, hour, minute) = unix_seconds_to_ymdhm(unix_seconds);
    format!("{day:02}/{month:02}/{year:04} {hour:02}:{minute:02}")
}

fn unix_seconds_to_ymdhm(unix_seconds: u64) -> (i32, u32, u32, u32, u32) {
    // Convert from days-since-1970 to a civil date using Howard Hinnant's
    // chrono algorithm. This avoids pulling in a date crate purely for a
    // file-modified timestamp.
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
    use super::{
        format_channels, format_duration_label, format_kind, format_sample_rate, format_size_label,
        unix_seconds_to_ymdhm,
    };
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn format_kind_recognises_common_extensions() {
        assert_eq!(format_kind(Some(Path::new("song.mp3"))), "MPEG audio file");
        assert_eq!(format_kind(Some(Path::new("song.flac"))), "FLAC audio file");
        assert_eq!(format_kind(None), "\u{2014}");
    }

    #[test]
    fn format_size_label_uses_binary_prefixes() {
        assert_eq!(format_size_label(Some(512)), "512 B");
        assert_eq!(format_size_label(Some(2_048)), "2.00 KB");
        assert_eq!(format_size_label(Some(5_242_880)), "5.00 MB");
        assert_eq!(format_size_label(None), "\u{2014}");
    }

    #[test]
    fn format_duration_label_includes_hours_when_needed() {
        assert_eq!(
            format_duration_label(Some(Duration::from_secs(245))),
            "4:05"
        );
        assert_eq!(
            format_duration_label(Some(Duration::from_secs(3_904))),
            "1:05:04"
        );
        assert_eq!(format_duration_label(None), "\u{2014}");
    }

    #[test]
    fn format_sample_rate_handles_common_rates() {
        assert_eq!(format_sample_rate(Some(44_100)), "44.1 kHz");
        assert_eq!(format_sample_rate(Some(48_000)), "48 kHz");
        assert_eq!(format_sample_rate(None), "\u{2014}");
    }

    #[test]
    fn format_channels_uses_human_labels() {
        assert_eq!(format_channels(Some(1)), "Mono");
        assert_eq!(format_channels(Some(2)), "Stereo");
        assert_eq!(format_channels(Some(6)), "6 channels");
        assert_eq!(format_channels(None), "\u{2014}");
    }

    #[test]
    fn unix_seconds_to_ymdhm_matches_known_dates() {
        assert_eq!(unix_seconds_to_ymdhm(0), (1970, 1, 1, 0, 0));
        // 2000-01-01 00:00:00 UTC
        assert_eq!(unix_seconds_to_ymdhm(946_684_800), (2000, 1, 1, 0, 0));
        // 2020-01-01 12:45:00 UTC
        assert_eq!(unix_seconds_to_ymdhm(1_577_882_700), (2020, 1, 1, 12, 45));
    }
}
