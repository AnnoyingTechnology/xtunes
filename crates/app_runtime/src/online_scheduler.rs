// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Paced background driver for network-bound retrievals.
//!
//! Mirrors [`crate::analysis_scheduler::AnalysisScheduler`] in shape
//! but targets remote work: tag enrichment via MusicBrainz, artwork
//! lookups via Cover Art Archive, and lyric pulls from LRClib. The
//! scheduler is intentionally conservative: capabilities are
//! missing-only (a track that already has embedded artwork or stored
//! lyrics is not contacted, tag fills never overwrite an existing
//! value), every completed attempt is stamped through
//! `track_online_status` so we do not re-fetch on every cycle, and
//! per-track pacing keeps the host polite even with the strict
//! per-host rate limits the HTTP client already enforces.
//!
//! Rate-limit handling: when any per-track attempt comes back with
//! [`sustain_metadata_remote::RemoteError::RateLimited`], the
//! capability that hit the limit is left un-stamped (so the next
//! batch picks it up after the HTTP client's per-host cool-down) and
//! the worker stops the current batch instead of running the
//! remaining tracks straight into the same wall.
//!
//! Lifecycle, command channel, and shutdown semantics are identical
//! to the analysis scheduler; see its docs for the longer rationale.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use sustain_domain::{FieldChange, MetadataChange, OnlineSettings, SyncedLyrics, Track, TrackId};
use sustain_library_store::{LibraryStore, OnlineCapabilities, OnlineContext};
use sustain_metadata::MetadataService;
use sustain_metadata_remote::{
    FetchedArtwork, FetchedLyrics, RemoteError, RemoteMetadataService, TrackMatch, TrackQuery,
};

use crate::artwork_fetcher::query_from_metadata;

/// How long the worker sleeps between two consecutive tracks. The
/// HTTP client's per-host rate limiter already prevents bursting
/// against any one provider; this extra pause holds the *cross*-host
/// rate down to something modest so background work does not saturate
/// the user's uplink during normal browsing.
const INTER_TRACK_PAUSE: Duration = Duration::from_millis(250);

/// How many tracks to fetch from the store per
/// `tracks_needing_online` query.
const BATCH_SIZE: usize = 16;

/// Short tag stored alongside synced lyrics so a future diagnostic UI
/// can answer "where did this come from?" without consulting logs.
const LRCLIB_SOURCE_TAG: &str = "lrclib";

/// Sink for progress updates emitted by the worker. The runtime wraps
/// this in an `async_channel` send so notifications surface on the
/// GTK main loop without the worker touching widgets directly.
pub type ProgressSink = Arc<dyn Fn(SchedulerProgress) + Send + Sync>;

/// Sink invoked once per track after the worker has mutated the
/// library store in a way the in-memory `library_tracks` copy needs
/// to see (a lyrics column update, a non-destructive tag fill). The
/// runtime wraps this in an `async_channel` send so the UI shell can
/// refresh that row on the main loop. Stays a no-op when no sink is
/// installed.
pub type TrackUpdatedSink = Arc<dyn Fn(TrackId) + Send + Sync>;

/// Wall-clock source recorded into `track_online_status.*_attempted_at_unix`.
pub type UnixClockFn = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Per-track progress signal. Same shape as the analysis scheduler so
/// the UI surface can use a shared widget treatment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerProgress {
    Tick {
        completed: u32,
        failed: u32,
        remaining: u32,
    },
    Idle {
        completed: u32,
        failed: u32,
    },
}

/// Bundle of dependencies the scheduler captures at start-up.
pub struct OnlineSchedulerConfig {
    pub remote_service: Arc<dyn RemoteMetadataService>,
    pub metadata_service: Arc<dyn MetadataService>,
    pub library_store: Arc<dyn LibraryStore>,
    pub progress: ProgressSink,
    /// Optional sink fired after each persisted track mutation so the
    /// runtime can refresh its in-memory `library_tracks` copy. `None`
    /// when the embedder does not care about live UI refreshes (tests,
    /// headless deployments).
    pub track_updated: Option<TrackUpdatedSink>,
    pub clock: UnixClockFn,
    pub initial_settings: OnlineSettings,
    pub library_path: Option<PathBuf>,
    pub provider_version: u32,
}

#[derive(Clone, Debug)]
enum SchedulerCommand {
    SettingsChanged(OnlineSettings),
    LibraryPathChanged(Option<PathBuf>),
    /// "Look for new work" — the library has grown or the user
    /// manually requested a re-run.
    Wake,
    Shutdown,
}

pub struct OnlineScheduler {
    sender: Sender<SchedulerCommand>,
    handle: Option<JoinHandle<()>>,
}

impl OnlineScheduler {
    pub fn start(config: OnlineSchedulerConfig) -> Self {
        let (sender, receiver) = mpsc::channel::<SchedulerCommand>();
        let handle = thread::Builder::new()
            .name("sustain-online-scheduler".to_owned())
            .spawn(move || worker_loop(receiver, config))
            .expect("spawn online scheduler thread");
        Self {
            sender,
            handle: Some(handle),
        }
    }

    pub fn update_settings(&self, settings: OnlineSettings) {
        let _ = self
            .sender
            .send(SchedulerCommand::SettingsChanged(settings));
    }

    pub fn set_library_path(&self, path: Option<PathBuf>) {
        let _ = self.sender.send(SchedulerCommand::LibraryPathChanged(path));
    }

    pub fn wake(&self) {
        let _ = self.sender.send(SchedulerCommand::Wake);
    }

    /// Send Shutdown, drop the sender, and join the worker. Blocks
    /// until the worker finishes the in-flight track (if any) and
    /// returns from its loop.
    pub fn shutdown(mut self) {
        let _ = self.sender.send(SchedulerCommand::Shutdown);
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for OnlineScheduler {
    fn drop(&mut self) {
        // Best-effort cleanup if `shutdown` was not called. Mirror the
        // analysis scheduler's discipline: do not join from Drop, since
        // Drop may run on the GTK main thread.
        let _ = self.sender.send(SchedulerCommand::Shutdown);
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
    }
}

fn worker_loop(receiver: mpsc::Receiver<SchedulerCommand>, config: OnlineSchedulerConfig) {
    let OnlineSchedulerConfig {
        remote_service,
        metadata_service,
        library_store,
        progress,
        track_updated,
        clock,
        initial_settings,
        library_path,
        provider_version,
    } = config;

    let mut state = WorkerState {
        settings: initial_settings,
        library_path,
        completed: 0,
        failed: 0,
    };

    loop {
        match drain_commands(&receiver, &mut state) {
            DrainOutcome::Shutdown => return,
            DrainOutcome::Continue => {}
        }

        let capabilities = effective_capabilities(&state.settings);
        if capabilities.is_empty() || state.library_path.is_none() {
            (progress)(SchedulerProgress::Idle {
                completed: state.completed,
                failed: state.failed,
            });
            state.completed = 0;
            state.failed = 0;
            match receiver.recv() {
                Ok(SchedulerCommand::Shutdown) | Err(_) => return,
                Ok(command) => apply_command(command, &mut state),
            }
            continue;
        }

        let pending =
            match library_store.tracks_needing_online(capabilities, provider_version, BATCH_SIZE) {
                Ok(ids) => ids,
                Err(_) => {
                    match receiver.recv() {
                        Ok(SchedulerCommand::Shutdown) | Err(_) => return,
                        Ok(command) => apply_command(command, &mut state),
                    }
                    continue;
                }
            };

        if pending.is_empty() {
            (progress)(SchedulerProgress::Idle {
                completed: state.completed,
                failed: state.failed,
            });
            state.completed = 0;
            state.failed = 0;
            match receiver.recv() {
                Ok(SchedulerCommand::Shutdown) | Err(_) => return,
                Ok(command) => apply_command(command, &mut state),
            }
            continue;
        }

        let library_path = match state.library_path.as_ref() {
            Some(path) => path.clone(),
            None => continue,
        };

        for track_id in pending {
            // Re-check between tracks so a toggle in Preferences stops
            // the loop within at most one track's worth of work.
            if let Some(command) = receiver.try_iter().next() {
                if matches!(command, SchedulerCommand::Shutdown) {
                    return;
                }
                apply_command(command, &mut state);
                if effective_capabilities(&state.settings).is_empty() {
                    break;
                }
            }

            let Ok(Some(track)) = library_store.track(track_id) else {
                continue;
            };
            let absolute_path = track.location.absolute_path(&library_path);
            let report = process_track(
                &track,
                &absolute_path,
                &state.settings,
                remote_service.as_ref(),
                metadata_service.as_ref(),
                library_store.as_ref(),
            );

            if matches!(report.outcome, ProcessOutcome::Succeeded)
                && let Some(notify) = track_updated.as_deref()
            {
                notify(track_id);
            }

            // Only stamp capabilities that actually completed — a
            // rate-limited attempt did not get to talk to the server,
            // so leaving it un-stamped means the next batch picks it
            // up again (after the HTTP client's per-host cool-down).
            if !report.attempted.is_empty() {
                let context = OnlineContext {
                    provider_version,
                    now_unix: (clock)(),
                };
                let _ = library_store.record_online_attempt(track_id, report.attempted, context);
            }

            match report.outcome {
                ProcessOutcome::Succeeded | ProcessOutcome::NoMatch => {
                    state.completed = state.completed.saturating_add(1);
                }
                ProcessOutcome::Failed | ProcessOutcome::RateLimited => {
                    state.failed = state.failed.saturating_add(1);
                }
            }

            let remaining = library_store
                .tracks_needing_online(
                    effective_capabilities(&state.settings),
                    provider_version,
                    BATCH_SIZE.saturating_mul(64),
                )
                .map(|ids| ids.len() as u32)
                .unwrap_or(0);
            (progress)(SchedulerProgress::Tick {
                completed: state.completed,
                failed: state.failed,
                remaining,
            });

            if matches!(report.outcome, ProcessOutcome::RateLimited) {
                // Stop the batch entirely on a rate-limit signal. The
                // HTTP client has already pushed the host's cool-down
                // forward, so even if we kept iterating we'd just sit
                // in `respect_rate_limit` for the same duration; this
                // way the worker drops back to the outer recv() and
                // resumes on the next nudge (library scan, settings
                // change, manual wake) without the cool-down also
                // blocking unrelated work.
                break;
            }

            thread::sleep(INTER_TRACK_PAUSE);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessOutcome {
    /// Provider returned data and the persist path succeeded for at
    /// least one capability requested.
    Succeeded,
    /// Every requested capability ran to completion and produced no
    /// new data. Still counted as a successful pass — the attempt
    /// timestamps for the capabilities we tried are stamped.
    NoMatch,
    /// A network or provider error occurred for at least one
    /// capability. Counted as failed for the UI summary; the
    /// capabilities that *did* complete are still stamped, the
    /// failing ones are stamped as well so we do not hammer a
    /// misbehaving provider every cycle.
    Failed,
    /// The server explicitly asked us to back off (HTTP 429/503).
    /// The HTTP client has already pushed the host's cool-down
    /// forward. The capabilities that hit the rate limit are *not*
    /// reported as attempted so the track stays eligible on the
    /// next pass; the worker also stops the current batch.
    RateLimited,
}

/// Per-track output of [`process_track`]: the overall outcome (used
/// for accounting and for the batch-break decision) plus the exact
/// set of capabilities that actually completed (used to decide what
/// to stamp into `track_online_status`). Anything that was rate-
/// limited is intentionally absent from `attempted` so the track
/// remains eligible for the next batch.
struct ProcessReport {
    outcome: ProcessOutcome,
    attempted: OnlineCapabilities,
}

fn process_track(
    track: &Track,
    absolute_path: &Path,
    settings: &OnlineSettings,
    remote_service: &dyn RemoteMetadataService,
    metadata_service: &dyn MetadataService,
    library_store: &dyn LibraryStore,
) -> ProcessReport {
    let query = query_from_metadata(&track.metadata);
    let mut any_success = false;
    let mut any_failure = false;
    let mut any_rate_limited = false;
    let mut attempted = OnlineCapabilities::none();

    // Tag enrichment runs first because the matched MusicBrainz
    // recording lets the subsequent artwork attempt walk releases
    // directly instead of re-identifying. We keep our own
    // `Option<TrackMatch>` so the matched result is reused, not
    // refetched.
    let mut cached_match: Option<TrackMatch> = None;

    if settings.tags {
        match attempt_tags(
            track,
            absolute_path,
            &query,
            remote_service,
            metadata_service,
            library_store,
            &mut cached_match,
        ) {
            AttemptOutcome::Succeeded => {
                any_success = true;
                attempted.tags = true;
            }
            AttemptOutcome::NoMatch => {
                attempted.tags = true;
            }
            AttemptOutcome::Failed => {
                any_failure = true;
                attempted.tags = true;
            }
            AttemptOutcome::RateLimited => {
                any_rate_limited = true;
            }
        }
    }

    if settings.artwork && !any_rate_limited {
        match attempt_artwork(
            track,
            absolute_path,
            &query,
            remote_service,
            metadata_service,
            cached_match.as_ref(),
        ) {
            AttemptOutcome::Succeeded => {
                any_success = true;
                attempted.artwork = true;
            }
            AttemptOutcome::NoMatch => {
                attempted.artwork = true;
            }
            AttemptOutcome::Failed => {
                any_failure = true;
                attempted.artwork = true;
            }
            AttemptOutcome::RateLimited => {
                any_rate_limited = true;
            }
        }
    }

    if settings.lyrics && !any_rate_limited {
        match attempt_lyrics(
            track,
            absolute_path,
            &query,
            remote_service,
            metadata_service,
            library_store,
        ) {
            AttemptOutcome::Succeeded => {
                any_success = true;
                attempted.lyrics = true;
            }
            AttemptOutcome::NoMatch => {
                attempted.lyrics = true;
            }
            AttemptOutcome::Failed => {
                any_failure = true;
                attempted.lyrics = true;
            }
            AttemptOutcome::RateLimited => {
                any_rate_limited = true;
            }
        }
    }

    let outcome = if any_rate_limited {
        ProcessOutcome::RateLimited
    } else if any_success {
        ProcessOutcome::Succeeded
    } else if any_failure {
        ProcessOutcome::Failed
    } else {
        ProcessOutcome::NoMatch
    };
    ProcessReport { outcome, attempted }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptOutcome {
    Succeeded,
    NoMatch,
    Failed,
    RateLimited,
}

/// Convert a remote-side error into the right attempt outcome.
/// `RateLimited` is handled distinctly so the scheduler can stop the
/// batch; every other error is a generic failure.
fn attempt_outcome_for_remote_error(error: &RemoteError) -> AttemptOutcome {
    if matches!(error, RemoteError::RateLimited { .. }) {
        AttemptOutcome::RateLimited
    } else {
        AttemptOutcome::Failed
    }
}

fn attempt_artwork(
    track: &Track,
    absolute_path: &Path,
    query: &TrackQuery,
    remote_service: &dyn RemoteMetadataService,
    metadata_service: &dyn MetadataService,
    cached_match: Option<&TrackMatch>,
) -> AttemptOutcome {
    // The "track already has embedded artwork" filter is enforced by
    // the SQL query (`tracks_needing_online` excludes rows with
    // `has_embedded_artwork = 1`); we trust that filter here and do
    // not re-probe the file at attempt time. If the bit is somehow
    // stale, the worst case is one unsolicited overwrite — but per
    // the policy the scanner is authoritative for this flag.
    let _ = track;

    let fetched: Option<FetchedArtwork> = match cached_match {
        Some(track_match) => match remote_service.fetch_artwork_for_match(track_match) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("Sustain: artwork fetch failed: {error}");
                return attempt_outcome_for_remote_error(&error);
            }
        },
        None => match remote_service.fetch_artwork(query) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("Sustain: artwork fetch failed: {error}");
                return attempt_outcome_for_remote_error(&error);
            }
        },
    };
    let Some(artwork) = fetched else {
        return AttemptOutcome::NoMatch;
    };
    if let Err(error) = metadata_service.write_artwork(absolute_path, Some(artwork.bytes)) {
        eprintln!(
            "Sustain: artwork tag write failed for {}: {error:?}",
            absolute_path.display()
        );
        return AttemptOutcome::Failed;
    }
    AttemptOutcome::Succeeded
}

fn attempt_lyrics(
    track: &Track,
    absolute_path: &Path,
    query: &TrackQuery,
    remote_service: &dyn RemoteMetadataService,
    metadata_service: &dyn MetadataService,
    library_store: &dyn LibraryStore,
) -> AttemptOutcome {
    let has_plain = track
        .metadata
        .lyrics
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_synced = library_store
        .load_synced_lyrics(track.id)
        .map(|stored| stored.is_some())
        .unwrap_or(false);

    if has_plain && has_synced {
        return AttemptOutcome::NoMatch;
    }

    let fetched: Option<FetchedLyrics> = match remote_service.fetch_lyrics(query) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Sustain: lyrics fetch failed: {error}");
            return attempt_outcome_for_remote_error(&error);
        }
    };
    let Some(fetched) = fetched else {
        return AttemptOutcome::NoMatch;
    };

    let mut wrote_anything = false;
    if !has_plain
        && let Some(plain) = fetched.plain
        && !plain.trim().is_empty()
    {
        let change = MetadataChange {
            lyrics: FieldChange::Set(plain.clone()),
            ..MetadataChange::default()
        };
        if let Err(error) = metadata_service.write_metadata(absolute_path, change) {
            eprintln!(
                "Sustain: lyrics tag write failed for {}: {error:?}",
                absolute_path.display()
            );
            return AttemptOutcome::Failed;
        }
        // Mirror into the tracks.lyrics column so the next read sees
        // the new value without another tag round-trip.
        let mut updated = track.clone();
        updated.metadata.lyrics = Some(plain);
        if let Err(error) = library_store.save_track(updated) {
            eprintln!(
                "Sustain: persist of lyrics column failed for {}: {error:?}",
                absolute_path.display()
            );
            return AttemptOutcome::Failed;
        }
        wrote_anything = true;
    }

    if !has_synced
        && let Some(synced_lrc) = fetched.synced_lrc.as_deref()
        && let Some(parsed) = SyncedLyrics::parse_lrc(synced_lrc)
    {
        if let Err(error) = library_store.record_synced_lyrics(track.id, &parsed, LRCLIB_SOURCE_TAG)
        {
            eprintln!("Sustain: synced lyrics persist failed: {error:?}");
            return AttemptOutcome::Failed;
        }
        wrote_anything = true;
    }

    if wrote_anything {
        AttemptOutcome::Succeeded
    } else {
        AttemptOutcome::NoMatch
    }
}

struct WorkerState {
    settings: OnlineSettings,
    library_path: Option<PathBuf>,
    completed: u32,
    failed: u32,
}

enum DrainOutcome {
    Continue,
    Shutdown,
}

fn drain_commands(
    receiver: &mpsc::Receiver<SchedulerCommand>,
    state: &mut WorkerState,
) -> DrainOutcome {
    while let Ok(command) = receiver.try_recv() {
        if matches!(command, SchedulerCommand::Shutdown) {
            return DrainOutcome::Shutdown;
        }
        apply_command(command, state);
    }
    DrainOutcome::Continue
}

fn apply_command(command: SchedulerCommand, state: &mut WorkerState) {
    match command {
        SchedulerCommand::SettingsChanged(settings) => {
            state.settings = settings;
        }
        SchedulerCommand::LibraryPathChanged(path) => {
            state.library_path = path;
        }
        SchedulerCommand::Wake | SchedulerCommand::Shutdown => {
            // Shutdown is handled at the caller; Wake has no side
            // effect beyond returning control to the loop top.
        }
    }
}

/// Project the user's `OnlineSettings` into `OnlineCapabilities` for
/// the storage layer. Every capability the scheduler actually
/// attempts must be reflected here so the attempt-stamping side stays
/// in sync with the work side.
fn effective_capabilities(settings: &OnlineSettings) -> OnlineCapabilities {
    OnlineCapabilities {
        artwork: settings.artwork,
        tags: settings.tags,
        lyrics: settings.lyrics,
    }
}

/// Non-destructive tag enrichment: identify the track through
/// MusicBrainz and fill in metadata fields that are currently empty.
/// Never overwrites existing data — per the persistence policy in
/// AGENTS.md, the library wins, and Sustain itself never re-imports
/// from external sources. Successful identifications are cached into
/// `cached_match` so the artwork attempt that runs next does not
/// need to re-identify the same track.
#[allow(clippy::too_many_arguments)]
fn attempt_tags(
    track: &Track,
    absolute_path: &Path,
    query: &TrackQuery,
    remote_service: &dyn RemoteMetadataService,
    metadata_service: &dyn MetadataService,
    library_store: &dyn LibraryStore,
    cached_match: &mut Option<TrackMatch>,
) -> AttemptOutcome {
    // Build the non-destructive change up front. If every field the
    // identifier could fill is already populated, we can skip the
    // remote call entirely.
    let mut change = MetadataChange::default();
    let mut change_releasable_fields = false;
    if track.metadata.title.is_none() {
        change_releasable_fields = true;
    }
    if track.metadata.artist.is_none() {
        change_releasable_fields = true;
    }
    if track.metadata.album.is_none()
        || track.metadata.year.is_none()
        || track.metadata.track_number.is_none()
        || track.metadata.track_total.is_none()
        || track.metadata.disc_number.is_none()
    {
        change_releasable_fields = true;
    }
    if !change_releasable_fields {
        return AttemptOutcome::NoMatch;
    }

    let matched = match remote_service.identify_track(query) {
        Ok(Some(value)) => value,
        Ok(None) => return AttemptOutcome::NoMatch,
        Err(error) => {
            eprintln!("Sustain: track identification failed: {error}");
            return attempt_outcome_for_remote_error(&error);
        }
    };

    if track.metadata.title.is_none()
        && let Some(value) = matched.title.as_deref()
        && !value.trim().is_empty()
    {
        change.title = FieldChange::Set(value.to_owned());
    }
    if track.metadata.artist.is_none()
        && let Some(value) = matched.artist.as_deref()
        && !value.trim().is_empty()
    {
        change.artist = FieldChange::Set(value.to_owned());
    }
    if let Some(release) = matched.releases.first() {
        if track.metadata.album.is_none()
            && let Some(value) = release.title.as_deref()
            && !value.trim().is_empty()
        {
            change.album = FieldChange::Set(value.to_owned());
        }
        if track.metadata.year.is_none()
            && let Some(year) = release.year
        {
            change.year = FieldChange::Set(year);
        }
        if track.metadata.track_number.is_none()
            && let Some(value) = release.track_number
        {
            change.track_number = FieldChange::Set(value);
        }
        if track.metadata.track_total.is_none()
            && let Some(value) = release.track_total
        {
            change.track_total = FieldChange::Set(value);
        }
        if track.metadata.disc_number.is_none()
            && let Some(value) = release.disc_number
        {
            change.disc_number = FieldChange::Set(value);
        }
    }

    *cached_match = Some(matched);

    if matches!(change.title, FieldChange::Unchanged)
        && matches!(change.artist, FieldChange::Unchanged)
        && matches!(change.album, FieldChange::Unchanged)
        && matches!(change.year, FieldChange::Unchanged)
        && matches!(change.track_number, FieldChange::Unchanged)
        && matches!(change.track_total, FieldChange::Unchanged)
        && matches!(change.disc_number, FieldChange::Unchanged)
    {
        // Identification succeeded but every field it could fill was
        // already present (e.g. user-supplied gaps that the match's
        // release happens not to cover). No write, but still
        // "attempted" — the SQL stamp keeps us from re-trying.
        return AttemptOutcome::NoMatch;
    }

    if let Err(error) = metadata_service.write_metadata(absolute_path, change.clone()) {
        eprintln!(
            "Sustain: tag enrichment write failed for {}: {error:?}",
            absolute_path.display()
        );
        return AttemptOutcome::Failed;
    }

    // Mirror the change into SQLite so the next read sees the new
    // values without another tag round-trip. apply_change preserves
    // existing fields by treating `Unchanged` as a no-op.
    let mut updated = track.clone();
    updated.metadata.apply_change(&change);
    if let Err(error) = library_store.save_track(updated) {
        eprintln!(
            "Sustain: tag enrichment persist failed for {}: {error:?}",
            absolute_path.display()
        );
        return AttemptOutcome::Failed;
    }

    AttemptOutcome::Succeeded
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::Write,
        path::Path,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU32, Ordering},
            mpsc as std_mpsc,
        },
        time::{Duration, Instant},
    };

    use sustain_domain::{
        MetadataChange, OnlineSettings, SyncedLyrics, Track, TrackLocation, TrackRelativePath,
    };
    use sustain_library_store::{InMemoryLibraryStore, LibraryStore, OnlineCapabilities, TrackId};
    use sustain_metadata::{MetadataError, MetadataResult, MetadataService};
    use sustain_metadata_remote::{
        FetchedArtwork, FetchedLyrics, RemoteError, RemoteMetadataService, RemoteResult,
        TrackMatch, TrackQuery,
    };
    use tempfile::TempDir;

    use super::{
        OnlineScheduler, OnlineSchedulerConfig, ProgressSink, SchedulerProgress, UnixClockFn,
    };

    fn touch_in(library_root: &Path, relative: &str) -> Track {
        let absolute = library_root.join(relative);
        if let Some(parent) = absolute.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        File::create(&absolute)
            .and_then(|mut f| f.write_all(b""))
            .expect("create file");
        let relative_path = TrackRelativePath::new(relative).expect("valid relative path");
        Track {
            id: TrackId::new(1).expect("non-zero"),
            location: TrackLocation::available(relative_path),
            content_hash: None,
            metadata: Default::default(),
            rating: Default::default(),
            statistics: Default::default(),
            file_size_bytes: None,
            has_embedded_artwork: None,
        }
    }

    fn fixed_clock(value: i64) -> UnixClockFn {
        Arc::new(move || value)
    }

    fn capturing_sink() -> (ProgressSink, std_mpsc::Receiver<SchedulerProgress>) {
        let (tx, rx) = std_mpsc::channel();
        let sink: ProgressSink = Arc::new(move |progress| {
            let _ = tx.send(progress);
        });
        (sink, rx)
    }

    fn wait_for(
        rx: &std_mpsc::Receiver<SchedulerProgress>,
        timeout: Duration,
        predicate: impl Fn(&SchedulerProgress) -> bool,
    ) -> SchedulerProgress {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let progress = rx
                .recv_timeout(remaining)
                .expect("scheduler progress within timeout");
            if predicate(&progress) {
                return progress;
            }
        }
    }

    /// Test double that returns canned fetch responses and records
    /// every call so assertions can verify "the scheduler did /
    /// did not contact the provider".
    #[derive(Default)]
    struct StubRemote {
        identify: Mutex<Option<RemoteResult<Option<TrackMatch>>>>,
        lyrics: Mutex<Option<RemoteResult<Option<FetchedLyrics>>>>,
        artwork: Mutex<Option<RemoteResult<Option<FetchedArtwork>>>>,
        artwork_for_match: Mutex<Option<RemoteResult<Option<FetchedArtwork>>>>,
        identify_calls: AtomicU32,
        lyrics_calls: AtomicU32,
        artwork_calls: AtomicU32,
        artwork_for_match_calls: AtomicU32,
    }

    impl StubRemote {
        fn with_lyrics(self, value: RemoteResult<Option<FetchedLyrics>>) -> Self {
            *self.lyrics.lock().expect("lock") = Some(value);
            self
        }
        fn with_artwork(self, value: RemoteResult<Option<FetchedArtwork>>) -> Self {
            *self.artwork.lock().expect("lock") = Some(value);
            self
        }
        fn with_identify(self, value: RemoteResult<Option<TrackMatch>>) -> Self {
            *self.identify.lock().expect("lock") = Some(value);
            self
        }
        fn with_artwork_for_match(self, value: RemoteResult<Option<FetchedArtwork>>) -> Self {
            *self.artwork_for_match.lock().expect("lock") = Some(value);
            self
        }
    }

    impl RemoteMetadataService for StubRemote {
        fn identify_track(&self, _query: &TrackQuery) -> RemoteResult<Option<TrackMatch>> {
            self.identify_calls.fetch_add(1, Ordering::SeqCst);
            self.identify
                .lock()
                .expect("lock")
                .clone()
                .unwrap_or(Ok(None))
        }
        fn fetch_artwork_for_match(
            &self,
            _track_match: &TrackMatch,
        ) -> RemoteResult<Option<FetchedArtwork>> {
            self.artwork_for_match_calls.fetch_add(1, Ordering::SeqCst);
            self.artwork_for_match
                .lock()
                .expect("lock")
                .clone()
                .unwrap_or(Ok(None))
        }
        fn fetch_artwork(&self, _query: &TrackQuery) -> RemoteResult<Option<FetchedArtwork>> {
            self.artwork_calls.fetch_add(1, Ordering::SeqCst);
            self.artwork
                .lock()
                .expect("lock")
                .clone()
                .unwrap_or(Ok(None))
        }
        fn fetch_lyrics(&self, _query: &TrackQuery) -> RemoteResult<Option<FetchedLyrics>> {
            self.lyrics_calls.fetch_add(1, Ordering::SeqCst);
            self.lyrics
                .lock()
                .expect("lock")
                .clone()
                .unwrap_or(Ok(None))
        }
    }

    /// Test double for the metadata service. Records every write so
    /// assertions can verify what the scheduler did.
    #[derive(Default)]
    struct StubMetadata {
        artwork_writes: Mutex<Vec<Option<Vec<u8>>>>,
        metadata_writes: Mutex<Vec<MetadataChange>>,
    }

    impl MetadataService for StubMetadata {
        fn read_metadata(&self, _path: &Path) -> MetadataResult<sustain_domain::TrackMetadata> {
            Ok(Default::default())
        }
        fn write_metadata(&self, _path: &Path, change: MetadataChange) -> MetadataResult<()> {
            self.metadata_writes.lock().expect("lock").push(change);
            Ok(())
        }
        fn read_rating(&self, _path: &Path) -> MetadataResult<Option<sustain_domain::Rating>> {
            Ok(None)
        }
        fn write_rating(
            &self,
            _path: &Path,
            _rating: sustain_domain::Rating,
        ) -> MetadataResult<()> {
            Err(MetadataError::WriteFailed)
        }
        fn write_artwork(&self, _path: &Path, artwork: Option<Vec<u8>>) -> MetadataResult<()> {
            self.artwork_writes.lock().expect("lock").push(artwork);
            Ok(())
        }
        fn read_artwork(&self, _path: &Path) -> MetadataResult<Option<Vec<u8>>> {
            Ok(None)
        }
    }

    fn track_with_metadata(library_root: &Path, relative: &str) -> Track {
        let mut t = touch_in(library_root, relative);
        t.metadata.artist = Some("Artist".to_owned());
        t.metadata.title = Some("Title".to_owned());
        t
    }

    #[test]
    fn scheduler_idles_with_no_capabilities_enabled() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store
            .save_track(track_with_metadata(temp.path(), "alpha.flac"))
            .expect("save");

        let remote = Arc::new(StubRemote::default());
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store,
            progress: sink,
            track_updated: None,
            clock: fixed_clock(0),
            initial_settings: OnlineSettings::default(),
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first progress");
        assert!(matches!(first, SchedulerProgress::Idle { .. }));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(remote.lyrics_calls.load(Ordering::SeqCst), 0);
        assert_eq!(remote.artwork_calls.load(Ordering::SeqCst), 0);

        scheduler.shutdown();
    }

    #[test]
    fn lyrics_capability_pulls_and_persists() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let track = track_with_metadata(temp.path(), "alpha.flac");
        store.save_track(track.clone()).expect("save");

        let remote = Arc::new(StubRemote::default().with_lyrics(Ok(Some(FetchedLyrics {
            plain: Some("Plain text".to_owned()),
            synced_lrc: Some("[00:01.50]Hello\n[00:03.00]World".to_owned()),
        }))));
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1_700_000_000),
            initial_settings: OnlineSettings {
                artwork: false,
                tags: false,
                lyrics: true,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { completed: 1, .. })
        });

        // Plain lyrics mirrored into tracks.lyrics and written via tag.
        let stored = store.track(track.id).expect("load").expect("present");
        assert_eq!(stored.metadata.lyrics.as_deref(), Some("Plain text"));
        assert_eq!(metadata.metadata_writes.lock().expect("lock").len(), 1);

        // Synced parsed and persisted.
        let synced = store
            .load_synced_lyrics(track.id)
            .expect("load")
            .expect("present");
        assert_eq!(synced.source, "lrclib");
        assert_eq!(
            synced.lyrics,
            SyncedLyrics::parse_lrc("[00:01.50]Hello\n[00:03.00]World").expect("parse")
        );

        // Attempt stamped — track no longer qualifies.
        assert!(
            store
                .tracks_needing_online(
                    OnlineCapabilities {
                        artwork: false,
                        tags: false,
                        lyrics: true,
                    },
                    1,
                    10,
                )
                .expect("query")
                .is_empty()
        );

        scheduler.shutdown();
    }

    #[test]
    fn lyrics_skipped_when_both_plain_and_synced_already_present() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let mut track = track_with_metadata(temp.path(), "alpha.flac");
        track.metadata.lyrics = Some("Existing".to_owned());
        store.save_track(track.clone()).expect("save");
        store
            .record_synced_lyrics(
                track.id,
                &SyncedLyrics::parse_lrc("[00:01.00]Already").expect("parse"),
                "test",
            )
            .expect("seed synced");

        let remote = Arc::new(StubRemote::default().with_lyrics(Ok(Some(FetchedLyrics {
            plain: Some("Should not overwrite".to_owned()),
            synced_lrc: Some("[00:02.00]Should not overwrite".to_owned()),
        }))));
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: false,
                tags: false,
                lyrics: true,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { .. })
        });

        // Remote should never have been called — both fields are
        // already present, so the worker short-circuits.
        assert_eq!(remote.lyrics_calls.load(Ordering::SeqCst), 0);
        // Existing values preserved.
        let stored = store.track(track.id).expect("load").expect("present");
        assert_eq!(stored.metadata.lyrics.as_deref(), Some("Existing"));
        let synced = store
            .load_synced_lyrics(track.id)
            .expect("load")
            .expect("present");
        assert_eq!(synced.source, "test");

        scheduler.shutdown();
    }

    #[test]
    fn artwork_capability_skips_when_embedded_present() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let mut track = track_with_metadata(temp.path(), "alpha.flac");
        // The scan-time bit is the contract here: when the file
        // already carries a picture, `tracks_needing_online` must
        // never offer this id for artwork even at a fresh
        // `provider_version`.
        track.has_embedded_artwork = Some(true);
        store.save_track(track.clone()).expect("save");

        let remote = Arc::new(StubRemote::default());
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: true,
                tags: false,
                lyrics: false,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        // The candidate list filters this id out at the SQL layer,
        // so the scheduler reaches Idle without ever invoking the
        // remote — no Tick is emitted.
        let _idle = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Idle { .. })
        });

        assert_eq!(
            remote.artwork_calls.load(Ordering::SeqCst),
            0,
            "track already has embedded artwork; no remote call needed"
        );
        assert!(metadata.artwork_writes.lock().expect("lock").is_empty());

        scheduler.shutdown();
    }

    #[test]
    fn artwork_capability_fetches_and_writes_when_missing() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let track = track_with_metadata(temp.path(), "alpha.flac");
        store.save_track(track.clone()).expect("save");

        let remote = Arc::new(StubRemote::default().with_artwork(Ok(Some(FetchedArtwork {
            bytes: vec![1, 2, 3, 4],
            release_mbid: "release".to_owned(),
        }))));
        let metadata = Arc::new(StubMetadata::default());
        // track.has_embedded_artwork left None → tracks_needing_online
        // treats it as "not yet scanned" and the artwork capability
        // still applies, so the scheduler asks the remote.
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: true,
                tags: false,
                lyrics: false,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { completed: 1, .. })
        });

        assert_eq!(remote.artwork_calls.load(Ordering::SeqCst), 1);
        let writes = metadata.artwork_writes.lock().expect("lock");
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].as_deref(), Some(&[1u8, 2, 3, 4][..]));

        scheduler.shutdown();
    }

    #[test]
    fn remote_error_records_attempt_and_is_not_retried() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let track = track_with_metadata(temp.path(), "alpha.flac");
        store.save_track(track.clone()).expect("save");

        let remote = Arc::new(StubRemote::default().with_lyrics(Err(RemoteError::Network)));
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: false,
                tags: false,
                lyrics: true,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { failed: 1, .. })
        });

        // Attempt stamped — track no longer qualifies even though the
        // provider errored.
        assert!(
            store
                .tracks_needing_online(
                    OnlineCapabilities {
                        artwork: false,
                        tags: false,
                        lyrics: true,
                    },
                    1,
                    10,
                )
                .expect("query")
                .is_empty()
        );

        scheduler.shutdown();
    }

    #[test]
    fn toggling_capabilities_off_stops_the_running_worker() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        for i in 0..16 {
            let relative = format!("track_{i:02}.flac");
            let mut t = track_with_metadata(temp.path(), &relative);
            t.id = TrackId::new(i + 1).expect("non-zero");
            store.save_track(t).expect("save");
        }

        let remote = Arc::new(StubRemote::default().with_lyrics(Ok(None)));
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: false,
                tags: false,
                lyrics: true,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _first = wait_for(
            &rx,
            Duration::from_secs(2),
            |progress| matches!(progress, SchedulerProgress::Tick { completed, .. } if *completed >= 1),
        );
        scheduler.update_settings(OnlineSettings::default());
        let _idle = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Idle { .. })
        });

        let before = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                1,
                100,
            )
            .expect("query")
            .len();
        std::thread::sleep(Duration::from_millis(500));
        let after = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                1,
                100,
            )
            .expect("query")
            .len();
        assert_eq!(
            before, after,
            "worker must stop attempting tracks once capabilities go to zero"
        );
        assert!(after > 0, "some tracks should still be un-attempted");

        scheduler.shutdown();
    }

    #[test]
    fn tags_capability_fills_only_missing_fields_and_caches_match_for_artwork() {
        use sustain_metadata_remote::{TrackMatchRelease, TrackMatchSource};

        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        // Seed a track that already has artist/title but no album/year.
        let mut track = track_with_metadata(temp.path(), "alpha.flac");
        track.metadata.artist = Some("Existing Artist".to_owned());
        track.metadata.title = Some("Existing Title".to_owned());
        store.save_track(track.clone()).expect("save");

        let track_match = TrackMatch {
            recording_mbid: "rec-mbid".to_owned(),
            title: Some("Other Title".to_owned()),
            artist: Some("Other Artist".to_owned()),
            releases: vec![TrackMatchRelease {
                release_mbid: "rel-mbid".to_owned(),
                release_group_mbid: None,
                title: Some("Filled Album".to_owned()),
                year: Some(2014),
                track_number: Some(3),
                track_total: Some(12),
                disc_number: Some(1),
            }],
            source: TrackMatchSource::MusicBrainzTags,
        };
        let remote = Arc::new(
            StubRemote::default()
                .with_identify(Ok(Some(track_match)))
                .with_artwork_for_match(Ok(Some(FetchedArtwork {
                    bytes: vec![0xAA, 0xBB],
                    release_mbid: "rel-mbid".to_owned(),
                }))),
        );
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata.clone(),
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: true,
                tags: true,
                lyrics: false,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { completed: 1, .. })
        });

        let stored = store.track(track.id).expect("load").expect("present");
        // Existing values were NOT overwritten.
        assert_eq!(stored.metadata.artist.as_deref(), Some("Existing Artist"));
        assert_eq!(stored.metadata.title.as_deref(), Some("Existing Title"));
        // Missing values were filled from the release.
        assert_eq!(stored.metadata.album.as_deref(), Some("Filled Album"));
        assert_eq!(stored.metadata.year, Some(2014));
        assert_eq!(stored.metadata.track_number, Some(3));
        assert_eq!(stored.metadata.track_total, Some(12));
        assert_eq!(stored.metadata.disc_number, Some(1));

        // Identification fires once, and the cached match drives the
        // artwork attempt — fetch_artwork (the unidentified path) is
        // never called.
        assert_eq!(remote.identify_calls.load(Ordering::SeqCst), 1);
        assert_eq!(remote.artwork_for_match_calls.load(Ordering::SeqCst), 1);
        assert_eq!(remote.artwork_calls.load(Ordering::SeqCst), 0);

        scheduler.shutdown();
    }

    #[test]
    fn track_updated_sink_fires_after_successful_lyrics_persist() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let track = track_with_metadata(temp.path(), "alpha.flac");
        store.save_track(track.clone()).expect("save");

        let remote = Arc::new(StubRemote::default().with_lyrics(Ok(Some(FetchedLyrics {
            plain: Some("Plain".to_owned()),
            synced_lrc: None,
        }))));
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let (notify_tx, notify_rx) = std_mpsc::channel::<TrackId>();
        let track_updated: super::TrackUpdatedSink = Arc::new(move |id| {
            let _ = notify_tx.send(id);
        });

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote,
            metadata_service: metadata,
            library_store: store.clone(),
            progress: sink,
            track_updated: Some(track_updated),
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: false,
                tags: false,
                lyrics: true,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { completed: 1, .. })
        });

        let observed = notify_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("track_updated sink fires after a successful persist");
        assert_eq!(observed, track.id);

        scheduler.shutdown();
    }

    #[test]
    fn rate_limited_lyrics_does_not_stamp_attempt_so_track_stays_eligible() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let track = track_with_metadata(temp.path(), "alpha.flac");
        store.save_track(track.clone()).expect("save");

        let remote = Arc::new(
            StubRemote::default().with_lyrics(Err(RemoteError::RateLimited {
                cool_down: Duration::from_secs(60),
            })),
        );
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata,
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: false,
                tags: false,
                lyrics: true,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        // The tick reports the failure exactly like any other; the
        // distinguishing behaviour lives in what didn't get written.
        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { failed: 1, .. })
        });

        // After the batch, the track must still qualify — a rate-limited
        // capability is never stamped, so the next pass picks it up
        // again once the HTTP client's per-host cool-down has elapsed.
        let still_pending = store
            .tracks_needing_online(
                OnlineCapabilities {
                    artwork: false,
                    tags: false,
                    lyrics: true,
                },
                1,
                10,
            )
            .expect("query");
        assert_eq!(
            still_pending,
            vec![track.id],
            "rate-limited track must remain eligible for the next batch"
        );

        scheduler.shutdown();
    }

    #[test]
    fn rate_limited_in_one_capability_still_stamps_other_completed_capabilities() {
        let temp = TempDir::new().expect("temp");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let track = track_with_metadata(temp.path(), "alpha.flac");
        store.save_track(track.clone()).expect("save");

        // tags runs first and succeeds (NoMatch), artwork then hits
        // a 429. After the batch, tags must be stamped (won't retry);
        // artwork must be left un-stamped (will retry after cool-down).
        let remote = Arc::new(
            StubRemote::default()
                .with_identify(Ok(None))
                .with_artwork(Err(RemoteError::RateLimited {
                    cool_down: Duration::from_secs(30),
                })),
        );
        let metadata = Arc::new(StubMetadata::default());
        let (sink, rx) = capturing_sink();

        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote.clone(),
            metadata_service: metadata,
            library_store: store.clone(),
            progress: sink,
            track_updated: None,
            clock: fixed_clock(1),
            initial_settings: OnlineSettings {
                artwork: true,
                tags: true,
                lyrics: false,
            },
            library_path: Some(temp.path().to_path_buf()),
            provider_version: 1,
        });

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { .. })
        });

        // tags is stamped → no longer a tags candidate.
        assert!(
            store
                .tracks_needing_online(
                    OnlineCapabilities {
                        artwork: false,
                        tags: true,
                        lyrics: false,
                    },
                    1,
                    10,
                )
                .expect("query")
                .is_empty(),
            "completed tags capability should be stamped"
        );
        // artwork is NOT stamped → still a candidate.
        assert_eq!(
            store
                .tracks_needing_online(
                    OnlineCapabilities {
                        artwork: true,
                        tags: false,
                        lyrics: false,
                    },
                    1,
                    10,
                )
                .expect("query"),
            vec![track.id],
            "rate-limited artwork capability must remain eligible"
        );

        scheduler.shutdown();
    }

    #[test]
    fn shutdown_returns_after_join() {
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let remote = Arc::new(StubRemote::default());
        let metadata = Arc::new(StubMetadata::default());
        let (sink, _rx) = capturing_sink();
        let scheduler = OnlineScheduler::start(OnlineSchedulerConfig {
            remote_service: remote,
            metadata_service: metadata,
            library_store: store,
            progress: sink,
            track_updated: None,
            clock: fixed_clock(0),
            initial_settings: OnlineSettings::default(),
            library_path: None,
            provider_version: 1,
        });
        let start = Instant::now();
        scheduler.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
