// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Central notification surface for user-facing status messages.
//!
//! Every user-visible status message — background task progress,
//! command outcomes, async tag write failures, artwork fetch results —
//! must flow through this module so the UI has a single, predictable
//! source to render. Feature code never pokes the status-bar widget
//! directly.
//!
//! Notifications come in two flavors:
//!
//! - [`NotificationKind::Persistent`] sticks until the producer
//!   explicitly dismisses it. Used for in-progress states (a scan
//!   running, an artwork fetch in flight); the widget paints them
//!   with a spinner and, when the kind says so, a Cancel button.
//!   Several persistents may stack — the most recent is shown; on
//!   dismissal the next one underneath returns to the surface.
//! - [`NotificationKind::Ephemeral`] auto-dismisses after
//!   [`EPHEMERAL_NOTIFICATION_DURATION`]. Used for one-shot outcomes.
//!   Ephemerals briefly preempt the persistent slot for visibility,
//!   then expire and the persistent comes back.
//!
//! The widget renders the head of `ephemeral_queue` if present, else
//! the back of `persistent_stack`. Both lists are pure data; the
//! widget is responsible for animation and timing.

use std::collections::VecDeque;
use std::time::Duration;

/// How long an Ephemeral stays at full opacity once it becomes the
/// displayed head. Product timing decision lives here as the single
/// source of truth; do not duplicate this value at call sites.
pub const EPHEMERAL_NOTIFICATION_DURATION: Duration = Duration::from_secs(4);

/// Duration of the slide+fade carousel transition the widget uses to
/// swap notifications. Co-located with the dismissal duration because
/// the two together describe one product-level timing budget.
pub const NOTIFICATION_TRANSITION: Duration = Duration::from_millis(250);

/// Runaway-safety guard on the ephemeral queue depth. We never evict
/// an un-expired notification at the head; this limit only triggers
/// on producers misbehaving, in which case we drop the newcomer (so
/// the user keeps the ability to read what is already queued).
pub const NOTIFICATION_QUEUE_HARD_CAP: usize = 15;

/// Monotonic, opaque identifier for a notification. Producers keep
/// hold of the id they get back from a push so they can later dismiss
/// the exact notification they created.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NotificationId(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NotificationCategory {
    LibraryScan,
    LibraryImport,
    LibraryConsolidation,
    ArtworkFetch,
    MetadataWrite,
    Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationKind {
    Ephemeral,
    Persistent { cancellable: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notification {
    pub id: NotificationId,
    pub category: NotificationCategory,
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub body: String,
}

/// Owns the live persistent stack and ephemeral queue. Held by
/// [`crate::ApplicationRuntime`]; feature code reaches it through the
/// runtime's typed push/dismiss helpers so the observer fires
/// uniformly on every mutation.
#[derive(Debug, Default)]
pub struct NotificationCenter {
    next_id: u64,
    persistent_stack: Vec<Notification>,
    ephemeral_queue: VecDeque<Notification>,
}

impl NotificationCenter {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            persistent_stack: Vec::new(),
            ephemeral_queue: VecDeque::new(),
        }
    }

    /// Currently-displayed persistent notification, or `None` when the
    /// stack is empty. The back of the stack wins so the most recent
    /// in-progress activity is what the user sees.
    pub fn current_persistent(&self) -> Option<&Notification> {
        self.persistent_stack.last()
    }

    pub fn current_ephemeral(&self) -> Option<&Notification> {
        self.ephemeral_queue.front()
    }

    pub fn ephemeral_queue(&self) -> &VecDeque<Notification> {
        &self.ephemeral_queue
    }

    pub fn persistent_stack(&self) -> &[Notification] {
        &self.persistent_stack
    }

    pub fn push_persistent(
        &mut self,
        category: NotificationCategory,
        severity: NotificationSeverity,
        body: String,
        cancellable: bool,
    ) -> NotificationId {
        let id = self.fresh_id();
        self.persistent_stack.push(Notification {
            id,
            category,
            kind: NotificationKind::Persistent { cancellable },
            severity,
            body,
        });
        id
    }

    /// Push an ephemeral, coalescing by category so a burst of similar
    /// outcomes does not stack up. The currently-displayed head is
    /// never preempted — it lives out its full timer regardless of
    /// what arrives next. If a queued (but not yet displayed)
    /// ephemeral in the same category exists, its body is replaced in
    /// place and its position preserved; otherwise the newcomer is
    /// appended to the tail.
    pub fn push_ephemeral(
        &mut self,
        category: NotificationCategory,
        severity: NotificationSeverity,
        body: String,
    ) -> NotificationId {
        let id = self.fresh_id();
        let notification = Notification {
            id,
            category,
            kind: NotificationKind::Ephemeral,
            severity,
            body,
        };

        // Skip the head: it is currently being read by the user and
        // its timer is already running. Anything past it is fair game
        // for in-place replacement so a burst of similar outcomes does
        // not stack up.
        if let Some(slot) = self
            .ephemeral_queue
            .iter_mut()
            .skip(1)
            .find(|queued| queued.category == category)
        {
            *slot = notification;
            return id;
        }

        if self.ephemeral_queue.len() >= NOTIFICATION_QUEUE_HARD_CAP {
            return id;
        }

        self.ephemeral_queue.push_back(notification);
        id
    }

    /// Remove the notification matching `id` from wherever it lives.
    /// No-op if the id is no longer present (already expired, already
    /// dismissed, never existed).
    pub fn dismiss(&mut self, id: NotificationId) {
        if let Some(index) = self
            .persistent_stack
            .iter()
            .position(|notification| notification.id == id)
        {
            self.persistent_stack.remove(index);
            return;
        }
        self.ephemeral_queue
            .retain(|notification| notification.id != id);
    }

    /// Drop the displayed ephemeral once its timer has elapsed. The
    /// widget calls this when it is ready to slide the next item in.
    pub fn expire_current_ephemeral(&mut self) -> Option<Notification> {
        self.ephemeral_queue.pop_front()
    }

    fn fresh_id(&mut self) -> NotificationId {
        // Wrap to 1 on overflow rather than 0 so an uninitialized id
        // is never accidentally valid in debug assertions.
        let id = NotificationId(self.next_id);
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        id
    }

    #[cfg(test)]
    fn __test_force_push_ephemeral(
        &mut self,
        category: NotificationCategory,
        body: String,
    ) -> NotificationId {
        let id = self.fresh_id();
        self.ephemeral_queue.push_back(Notification {
            id,
            category,
            kind: NotificationKind::Ephemeral,
            severity: NotificationSeverity::Info,
            body,
        });
        id
    }
}

// User-facing message catalogue. Lives in `app_runtime` so the runtime
// can populate `Notification::body` at the same point it transitions
// its task state. The widget renders the string raw, with no
// case-by-case knowledge of what it means.

use crate::{
    ApplicationRuntimeError, LibraryConsolidationSummary, LibraryImportSummary, LibraryScanSummary,
};

pub fn library_scan_running_text() -> String {
    "Scanning library...".to_owned()
}

pub fn library_import_running_text() -> String {
    "Adding tracks...".to_owned()
}

pub fn library_consolidation_running_text() -> String {
    "Organizing library...".to_owned()
}

pub fn library_scan_outcome_text(summary: &LibraryScanSummary) -> String {
    if summary.cancelled {
        return format!(
            "Scan stopped: {} {} indexed.",
            summary.scanned_tracks,
            pluralize(summary.scanned_tracks, "track", "tracks"),
        );
    }
    format!(
        "Scan complete: {} {}, {} missing, {} failed",
        summary.scanned_tracks,
        pluralize(summary.scanned_tracks, "track", "tracks"),
        summary.missing_tracks,
        summary.failed_files,
    )
}

pub fn library_import_outcome_text(summary: &LibraryImportSummary) -> String {
    if summary.cancelled {
        return format!(
            "Import stopped: {} added before cancel.",
            summary.imported_tracks
        );
    }
    match (
        summary.imported_tracks,
        summary.duplicate_files,
        summary.discovered_files,
    ) {
        (0, 0, 0) => "No audio files were found.".to_owned(),
        (imported, 0, _) => format!("{imported} tracks added."),
        (imported, duplicates, _) => {
            format!("{imported} tracks added, {duplicates} duplicates skipped.")
        }
    }
}

pub fn library_consolidation_outcome_text(summary: &LibraryConsolidationSummary) -> String {
    if summary.cancelled {
        return format!(
            "Library organization stopped: {} moved, {} pending.",
            summary.moved_tracks,
            summary.planned_tracks.saturating_sub(summary.moved_tracks)
        );
    }
    format!(
        "Library organized: {} moved, {} already organized, {} missing.",
        summary.moved_tracks, summary.already_organized_tracks, summary.missing_tracks
    )
}

/// Outcome string emitted after the user changes their library path.
/// `newly_missing` is the number of tracks whose file did not resolve
/// under the new root; `total` is the size of the persisted library.
/// Both reflect SQLite state immediately after the re-stat pass.
pub fn library_path_change_outcome_text(newly_missing: usize, total: usize) -> String {
    if total == 0 {
        return "Library folder updated.".to_owned();
    }
    if newly_missing == 0 {
        return format!(
            "Library folder updated: all {} {} found.",
            total,
            pluralize(total, "track", "tracks"),
        );
    }
    format!(
        "Library folder updated: {} of {} {} not found at the new location.",
        newly_missing,
        total,
        pluralize(total, "track", "tracks"),
    )
}

pub fn runtime_error_text(error: &ApplicationRuntimeError) -> &'static str {
    match error {
        ApplicationRuntimeError::BackgroundTaskRunning => {
            "Another background task is already running."
        }
        ApplicationRuntimeError::LibraryScanFailed => "The selected folder could not be scanned.",
        ApplicationRuntimeError::LibraryConsolidationFailed => {
            "The library could not be organized."
        }
        ApplicationRuntimeError::LibraryServicesUnavailable => {
            "Library scanning is not available in this build."
        }
        ApplicationRuntimeError::LibraryStoreFailed => "The library database could not be updated.",
        ApplicationRuntimeError::LibraryPathUnavailable => "Choose a library folder first.",
        ApplicationRuntimeError::LibraryImportFailed => {
            "The files could not be added to the library."
        }
        ApplicationRuntimeError::MetadataWriteFailed => "The track metadata could not be updated.",
        ApplicationRuntimeError::InvalidPlaylistName => "The playlist name is not valid.",
        ApplicationRuntimeError::InvalidPlaylistFolderName => "The folder name is not valid.",
        ApplicationRuntimeError::InvalidSmartPlaylistName => {
            "The smart playlist name is not valid."
        }
        ApplicationRuntimeError::InvalidSmartPlaylistRules => {
            "A smart playlist needs at least one rule."
        }
        ApplicationRuntimeError::PlaylistEntryNotFound
        | ApplicationRuntimeError::PlaylistNotFound => "The playlist could not be updated.",
        ApplicationRuntimeError::PlaylistFolderNotFound => {
            "The playlist folder could not be updated."
        }
        ApplicationRuntimeError::PlaylistFolderWouldCycle => {
            "A folder cannot be moved inside itself."
        }
        ApplicationRuntimeError::SmartPlaylistNotFound => {
            "The smart playlist could not be updated."
        }
        ApplicationRuntimeError::SettingsLoadFailed
        | ApplicationRuntimeError::SettingsSaveFailed => "The library path could not be saved.",
        ApplicationRuntimeError::PlaybackFailed
        | ApplicationRuntimeError::PlaybackServiceUnavailable => "Playback is not available.",
        ApplicationRuntimeError::TrackUnavailable => "Track file is missing.",
        ApplicationRuntimeError::TrackTrashFailed => "The track could not be moved to trash.",
        ApplicationRuntimeError::ArtworkFetchingUnavailable => {
            "Remote artwork retrieval is not available in this build."
        }
        ApplicationRuntimeError::UnsupportedCommand(_) => "This action is not available yet.",
    }
}

fn pluralize(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod text_tests {
    use super::*;

    #[test]
    fn scan_outcome_mentions_missing_and_failed_counts() {
        let summary = LibraryScanSummary {
            scanned_tracks: 10,
            missing_tracks: 2,
            failed_files: 1,
            ..LibraryScanSummary::default()
        };
        assert_eq!(
            library_scan_outcome_text(&summary),
            "Scan complete: 10 tracks, 2 missing, 1 failed"
        );
    }

    #[test]
    fn scan_outcome_reports_partial_count_after_cancellation() {
        let summary = LibraryScanSummary {
            scanned_tracks: 42,
            cancelled: true,
            ..LibraryScanSummary::default()
        };
        assert_eq!(
            library_scan_outcome_text(&summary),
            "Scan stopped: 42 tracks indexed."
        );
    }

    #[test]
    fn runtime_error_text_maps_metadata_write_failed() {
        assert_eq!(
            runtime_error_text(&ApplicationRuntimeError::MetadataWriteFailed),
            "The track metadata could not be updated."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(s: &str) -> String {
        s.to_owned()
    }

    #[test]
    fn ephemeral_pushes_append_to_the_tail() {
        let mut center = NotificationCenter::new();
        let first = center.push_ephemeral(
            NotificationCategory::LibraryScan,
            NotificationSeverity::Info,
            body("Scan complete: 1 track"),
        );
        let second = center.push_ephemeral(
            NotificationCategory::ArtworkFetch,
            NotificationSeverity::Info,
            body("Artwork updated."),
        );

        let queue: Vec<_> = center.ephemeral_queue().iter().map(|n| n.id).collect();
        assert_eq!(queue, vec![first, second]);
    }

    #[test]
    fn same_category_ephemeral_replaces_in_queue_without_preempting_head() {
        let mut center = NotificationCenter::new();
        let head = center.push_ephemeral(
            NotificationCategory::LibraryImport,
            NotificationSeverity::Info,
            body("Imported 1 track"),
        );
        let stale = center.push_ephemeral(
            NotificationCategory::LibraryImport,
            NotificationSeverity::Info,
            body("Imported 5 tracks"),
        );
        let _ = stale;
        let fresh = center.push_ephemeral(
            NotificationCategory::LibraryImport,
            NotificationSeverity::Info,
            body("Imported 12 tracks"),
        );

        assert_eq!(center.ephemeral_queue().len(), 2);
        let head_now = center.current_ephemeral().expect("head present");
        assert_eq!(head_now.id, head);
        assert_eq!(head_now.body, "Imported 1 track");
        let tail_now = center.ephemeral_queue().back().expect("tail present");
        assert_eq!(tail_now.id, fresh);
        assert_eq!(tail_now.body, "Imported 12 tracks");
    }

    #[test]
    fn unrelated_categories_are_not_coalesced() {
        let mut center = NotificationCenter::new();
        center.push_ephemeral(
            NotificationCategory::LibraryScan,
            NotificationSeverity::Info,
            body("Scan complete"),
        );
        center.push_ephemeral(
            NotificationCategory::ArtworkFetch,
            NotificationSeverity::Info,
            body("Artwork updated."),
        );
        center.push_ephemeral(
            NotificationCategory::LibraryScan,
            NotificationSeverity::Info,
            body("Scan complete again"),
        );

        let bodies: Vec<_> = center
            .ephemeral_queue()
            .iter()
            .map(|n| n.body.clone())
            .collect();
        // Head (LibraryScan) is untouched. The second push in the
        // LibraryScan category replaces the queued (non-head) entry —
        // but here there is no queued LibraryScan entry, so it appends.
        assert_eq!(
            bodies,
            vec![
                "Scan complete".to_owned(),
                "Artwork updated.".to_owned(),
                "Scan complete again".to_owned(),
            ]
        );
    }

    #[test]
    fn dismiss_removes_persistent_by_id() {
        let mut center = NotificationCenter::new();
        let id = center.push_persistent(
            NotificationCategory::LibraryScan,
            NotificationSeverity::Info,
            body("Scanning library..."),
            true,
        );
        assert!(center.current_persistent().is_some());
        center.dismiss(id);
        assert!(center.current_persistent().is_none());
    }

    #[test]
    fn newer_persistent_displaces_older_until_dismissed() {
        let mut center = NotificationCenter::new();
        let scan = center.push_persistent(
            NotificationCategory::LibraryScan,
            NotificationSeverity::Info,
            body("Scanning library..."),
            true,
        );
        let fetch = center.push_persistent(
            NotificationCategory::ArtworkFetch,
            NotificationSeverity::Info,
            body("Fetching artwork..."),
            false,
        );

        assert_eq!(
            center.current_persistent().map(|n| n.id),
            Some(fetch),
            "back of stack wins"
        );

        center.dismiss(fetch);
        assert_eq!(
            center.current_persistent().map(|n| n.id),
            Some(scan),
            "scan returns to surface once fetch dismisses",
        );
    }

    #[test]
    fn expire_current_ephemeral_pops_the_head() {
        let mut center = NotificationCenter::new();
        let first = center.push_ephemeral(
            NotificationCategory::LibraryScan,
            NotificationSeverity::Info,
            body("First"),
        );
        let second = center.push_ephemeral(
            NotificationCategory::ArtworkFetch,
            NotificationSeverity::Info,
            body("Second"),
        );

        let expired = center.expire_current_ephemeral().expect("had a head");
        assert_eq!(expired.id, first);
        assert_eq!(center.current_ephemeral().map(|n| n.id), Some(second));
    }

    #[test]
    fn hard_cap_drops_newcomers_rather_than_evicting_unexpired_entries() {
        // Replace-by-category keeps the queue tiny under normal use,
        // so the cap is unreachable through the regular push path.
        // The cap is defence in depth against pathological producers;
        // we prove its behavior by force-stuffing the queue past
        // coalescing.
        let mut center = NotificationCenter::new();
        for index in 0..NOTIFICATION_QUEUE_HARD_CAP {
            center
                .__test_force_push_ephemeral(NotificationCategory::Command, format!("msg {index}"));
        }
        assert_eq!(center.ephemeral_queue().len(), NOTIFICATION_QUEUE_HARD_CAP);

        let _ = center.push_ephemeral(
            NotificationCategory::ArtworkFetch,
            NotificationSeverity::Info,
            body("overflow"),
        );
        assert_eq!(center.ephemeral_queue().len(), NOTIFICATION_QUEUE_HARD_CAP);
        assert!(
            !center
                .ephemeral_queue()
                .iter()
                .any(|n| n.body == "overflow"),
            "newest is dropped when the cap is hit",
        );
    }
}
