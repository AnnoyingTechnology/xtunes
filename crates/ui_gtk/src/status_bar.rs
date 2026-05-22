use gtk::prelude::*;

use super::{BackgroundTaskStatus, LibraryScanSummary, STATUS_BAR_HEIGHT};
use crate::track_table::TrackTableRow;

#[derive(Clone)]
pub(crate) struct StatusBar {
    root: gtk::CenterBox,
    summary: gtk::Label,
    task_box: gtk::Box,
    task_spinner: gtk::Spinner,
    task_label: gtk::Label,
}

impl StatusBar {
    pub(crate) fn new(library_tracks: &[TrackTableRow]) -> Self {
        let root = gtk::CenterBox::new();
        root.add_css_class("status-bar");
        root.set_height_request(STATUS_BAR_HEIGHT);
        root.set_hexpand(true);

        let summary = gtk::Label::new(None);
        summary.set_xalign(0.5);

        let task_spinner = gtk::Spinner::new();
        task_spinner.add_css_class("task-status-spinner");
        task_spinner.set_size_request(14, 14);

        let task_label = gtk::Label::new(None);
        task_label.add_css_class("task-status-label");
        task_label.set_xalign(1.0);

        let task_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        task_box.add_css_class("task-status");
        task_box.set_valign(gtk::Align::Center);
        task_box.set_halign(gtk::Align::End);
        task_box.append(&task_spinner);
        task_box.append(&task_label);

        root.set_center_widget(Some(&summary));
        root.set_end_widget(Some(&task_box));

        let status_bar = Self {
            root,
            summary,
            task_box,
            task_spinner,
            task_label,
        };
        status_bar.update_summary(library_tracks);
        status_bar.update_task(&BackgroundTaskStatus::Idle);
        status_bar
    }

    pub(crate) fn widget(&self) -> gtk::CenterBox {
        self.root.clone()
    }

    pub(crate) fn update_summary(&self, library_tracks: &[TrackTableRow]) {
        let duration_seconds = library_tracks
            .iter()
            .map(|track| track.duration_seconds)
            .sum();
        let size_bytes = library_tracks
            .iter()
            .map(|track| track.file_size_bytes)
            .sum();

        self.summary.set_text(&library_status_text(
            library_tracks.len(),
            duration_seconds,
            size_bytes,
        ));
    }

    pub(crate) fn update_task(&self, status: &BackgroundTaskStatus) {
        self.task_box
            .set_visible(!matches!(status, BackgroundTaskStatus::Idle));
        self.task_spinner
            .set_visible(matches!(status, BackgroundTaskStatus::LibraryScanRunning));
        self.task_spinner
            .set_spinning(matches!(status, BackgroundTaskStatus::LibraryScanRunning));
        self.task_label.set_text(&task_status_text(status));
    }
}

fn task_status_text(status: &BackgroundTaskStatus) -> String {
    match status {
        BackgroundTaskStatus::Idle => String::new(),
        BackgroundTaskStatus::LibraryScanRunning => "Scanning library...".to_owned(),
        BackgroundTaskStatus::LibraryScanCompleted(summary) => scan_summary_text(summary),
        BackgroundTaskStatus::LibraryScanFailed(error) => match error {
            super::ApplicationRuntimeError::BackgroundTaskRunning => {
                "Another background task is already running.".to_owned()
            }
            super::ApplicationRuntimeError::LibraryScanFailed => {
                "The selected folder could not be scanned.".to_owned()
            }
            super::ApplicationRuntimeError::LibraryServicesUnavailable => {
                "Library scanning is not available in this build.".to_owned()
            }
            super::ApplicationRuntimeError::LibraryStoreFailed => {
                "The library database could not be updated.".to_owned()
            }
            super::ApplicationRuntimeError::SettingsLoadFailed
            | super::ApplicationRuntimeError::SettingsSaveFailed => {
                "The library path could not be saved.".to_owned()
            }
            super::ApplicationRuntimeError::PlaybackFailed
            | super::ApplicationRuntimeError::PlaybackServiceUnavailable
            | super::ApplicationRuntimeError::TrackUnavailable => {
                "Playback is not available.".to_owned()
            }
            super::ApplicationRuntimeError::TrackTrashFailed => {
                "The track could not be moved to trash.".to_owned()
            }
        },
    }
}

fn scan_summary_text(summary: &LibraryScanSummary) -> String {
    format!(
        "Scan complete: {} {}, {} missing, {} failed",
        summary.scanned_tracks,
        pluralize(summary.scanned_tracks, "track", "tracks"),
        summary.missing_tracks,
        summary.failed_files,
    )
}

pub(crate) fn library_status_text(
    track_count: usize,
    duration_seconds: u64,
    size_bytes: u64,
) -> String {
    format!(
        "{} {}, {}, {}",
        track_count,
        pluralize(track_count, "song", "songs"),
        duration_text(duration_seconds),
        file_size_text(size_bytes),
    )
}

fn duration_text(duration_seconds: u64) -> String {
    let hours = duration_seconds / 3_600;
    if hours >= 24 {
        let days = hours / 24;
        format!("{} {}", days, pluralize(days as usize, "day", "days"))
    } else {
        format!("{} {}", hours, pluralize(hours as usize, "hour", "hours"))
    }
}

fn file_size_text(size_bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;

    if size_bytes >= GB {
        format!("{} GB", size_bytes / GB)
    } else {
        format!("{} MB", size_bytes / MB)
    }
}

fn pluralize(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_status_uses_hours_and_megabytes_for_small_libraries() {
        assert_eq!(
            library_status_text(2, 7_200, 250_000_000),
            "2 songs, 2 hours, 250 MB"
        );
    }

    #[test]
    fn library_status_uses_days_and_gigabytes_for_large_libraries() {
        assert_eq!(
            library_status_text(1, 172_800, 3_000_000_000),
            "1 song, 2 days, 3 GB"
        );
    }

    #[test]
    fn scan_summary_text_mentions_missing_and_failed_counts() {
        let summary = LibraryScanSummary {
            scanned_tracks: 10,
            missing_tracks: 2,
            failed_files: 1,
            ..LibraryScanSummary::default()
        };

        assert_eq!(
            scan_summary_text(&summary),
            "Scan complete: 10 tracks, 2 missing, 1 failed"
        );
    }
}
