// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Paced background driver for `sustain_analysis::analyze`.
//!
//! Owns one named worker thread that pulls tracks-needing-analysis
//! from the [`LibraryStore`] and feeds them through a caller-supplied
//! analyzer function (production: `sustain_analysis::analyze`; tests:
//! an in-memory stub). Records every attempt — success or failure —
//! through the store so the scheduler is idempotent across restarts.
//!
//! Lifecycle:
//!
//! * [`AnalysisScheduler::start`] spawns the worker.
//! * Settings/library-path changes and "new tracks landed" pokes are
//!   delivered over an mpsc command channel; the worker also wakes
//!   itself between tracks to drain pending commands.
//! * [`AnalysisScheduler::shutdown`] drops the sender and joins.
//!   Shutdown does NOT cancel an in-flight analysis — the worker
//!   finishes the current track (a few seconds at most) and exits at
//!   the next loop iteration. Cancellation mid-DSP would require
//!   threading an `AtomicBool` through `sustain_analysis::analyze`,
//!   which we can add later if the latency becomes user-perceptible.
//!
//! Pacing: a short sleep between tracks (see `INTER_TRACK_PAUSE`)
//! keeps the CPU well below saturation on a 16-thread laptop while
//! still draining a 9000-track library in a few hours of background
//! work. The pause is the only knob today; if the user can hear the
//! load during playback we can plumb a configurable pace through
//! [`AnalysisSchedulerConfig`].

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use sustain_analysis::{AnalysisError, AnalysisOptions, TrackAnalysis};
use sustain_domain::AnalysisSettings;
use sustain_library_store::{
    AnalysisAttemptContext, AnalysisCapabilities, AnalysisRunContext, LibraryStore,
};

/// How long the worker sleeps between two consecutive analyses. Keeps
/// the average CPU load well below saturation so playback and UI
/// interaction stay responsive on the maintainer's machines.
const INTER_TRACK_PAUSE: Duration = Duration::from_millis(100);

/// How many tracks to fetch from the store per `tracks_needing_analysis`
/// query. Small enough that capability changes propagate within a few
/// tracks, large enough that we are not querying the database between
/// every track.
const BATCH_SIZE: usize = 16;

/// The DSP function the worker calls per track. Production wires this
/// to `sustain_analysis::analyze`; tests substitute a closure that
/// returns canned `TrackAnalysis` values.
pub type AnalyzerFn =
    Arc<dyn Fn(&Path, AnalysisOptions) -> Result<TrackAnalysis, AnalysisError> + Send + Sync>;

/// Sink for progress updates emitted by the worker thread. The runtime
/// wraps this in an `async_channel` send so notifications surface on
/// the GTK main loop without the worker touching widgets directly.
pub type ProgressSink = Arc<dyn Fn(SchedulerProgress) + Send + Sync>;

/// Source for the current wall-clock unix timestamp recorded into the
/// `track_analysis.*_attempted_at_unix` columns. Injected so tests can
/// run with a deterministic clock; production passes a closure backed
/// by `SystemTime::now()`.
pub type UnixClockFn = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Per-track progress signal. The widget aggregates these into the
/// notification text without caring about the underlying ids.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerProgress {
    /// At least one track has been processed since the last summary.
    /// `completed` and `failed` are batch-running totals; `remaining`
    /// is the most-recent "still pending" estimate from the store.
    Tick {
        completed: u32,
        failed: u32,
        remaining: u32,
    },
    /// The worker has caught up: either the queue is empty or every
    /// requested capability is currently disabled. UI side: dismiss
    /// the persistent "analysing" notification, emit an ephemeral
    /// summary if `completed + failed > 0`.
    Idle { completed: u32, failed: u32 },
}

/// Bundle of dependencies the scheduler captures at start-up. Grouped
/// so the trait method signature stays manageable and so tests can
/// build a config struct once and reuse it across cases.
pub struct AnalysisSchedulerConfig {
    pub analyzer: AnalyzerFn,
    pub progress: ProgressSink,
    pub clock: UnixClockFn,
    pub library_store: Arc<dyn LibraryStore>,
    pub initial_settings: AnalysisSettings,
    pub library_path: Option<PathBuf>,
    pub analyzer_version: u32,
    pub analysis_options: AnalysisOptions,
}

#[derive(Clone, Debug)]
enum SchedulerCommand {
    SettingsChanged(AnalysisSettings),
    LibraryPathChanged(Option<PathBuf>),
    /// "Look for new work" — the store may have grown (library scan
    /// added tracks, or the user manually requested re-analysis).
    Wake,
    Shutdown,
}

pub struct AnalysisScheduler {
    sender: Sender<SchedulerCommand>,
    handle: Option<JoinHandle<()>>,
}

impl AnalysisScheduler {
    pub fn start(config: AnalysisSchedulerConfig) -> Self {
        let (sender, receiver) = mpsc::channel::<SchedulerCommand>();
        let handle = thread::Builder::new()
            .name("sustain-analysis-scheduler".to_owned())
            .spawn(move || worker_loop(receiver, config))
            .expect("spawn analysis scheduler thread");
        Self {
            sender,
            handle: Some(handle),
        }
    }

    pub fn update_settings(&self, settings: AnalysisSettings) {
        // Channel send only fails on a closed channel, which only
        // happens after `shutdown` — by which point the runtime should
        // not be issuing more updates. Drop silently in that case.
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
        // Drop the live sender so even if Shutdown was lost (impossible
        // with mpsc, but defensive) the worker's `recv` returns Err.
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AnalysisScheduler {
    fn drop(&mut self) {
        // Best-effort cleanup if `shutdown` was not called. Same
        // discipline as the other workers: don't block on join in Drop,
        // since Drop may be running on the GTK main thread.
        let _ = self.sender.send(SchedulerCommand::Shutdown);
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
    }
}

fn worker_loop(receiver: mpsc::Receiver<SchedulerCommand>, config: AnalysisSchedulerConfig) {
    let AnalysisSchedulerConfig {
        analyzer,
        progress,
        clock,
        library_store,
        initial_settings,
        library_path,
        analyzer_version,
        analysis_options,
    } = config;

    let mut state = WorkerState {
        settings: initial_settings,
        library_path,
        completed: 0,
        failed: 0,
    };

    loop {
        // Drain any pending commands without blocking so settings
        // changes apply between batches, not between tracks.
        match drain_commands(&receiver, &mut state) {
            DrainOutcome::Shutdown => return,
            DrainOutcome::Continue => {}
        }

        let capabilities = capabilities_from(&state.settings);
        if capabilities.is_empty() || state.library_path.is_none() {
            // Nothing to do; emit Idle so the UI can dismiss any
            // persistent "analysing" notification, then block for the
            // next command.
            (progress)(SchedulerProgress::Idle {
                completed: state.completed,
                failed: state.failed,
            });
            state.completed = 0;
            state.failed = 0;
            match receiver.recv() {
                Ok(SchedulerCommand::Shutdown) | Err(_) => return,
                Ok(command) => {
                    apply_command(command, &mut state);
                }
            }
            continue;
        }

        let pending =
            match library_store.tracks_needing_analysis(capabilities, analyzer_version, BATCH_SIZE)
            {
                Ok(ids) => ids,
                Err(_) => {
                    // A store error here would be alarming; wait for an
                    // explicit Wake/SettingsChanged before retrying so we
                    // do not hot-loop on a broken database.
                    match receiver.recv() {
                        Ok(SchedulerCommand::Shutdown) | Err(_) => return,
                        Ok(command) => {
                            apply_command(command, &mut state);
                        }
                    }
                    continue;
                }
            };

        if pending.is_empty() {
            // Caught up: emit Idle summary and wait for next nudge.
            (progress)(SchedulerProgress::Idle {
                completed: state.completed,
                failed: state.failed,
            });
            state.completed = 0;
            state.failed = 0;
            match receiver.recv() {
                Ok(SchedulerCommand::Shutdown) | Err(_) => return,
                Ok(command) => {
                    apply_command(command, &mut state);
                }
            }
            continue;
        }

        let library_path = match state.library_path.as_ref() {
            Some(path) => path.clone(),
            None => continue, // re-checked at the top of the loop
        };

        for track_id in pending {
            // Re-check settings between every track so a toggle in the
            // Preferences window stops the loop within at most one
            // track's analysis time.
            if let Some(command) = receiver.try_iter().next() {
                if matches!(command, SchedulerCommand::Shutdown) {
                    return;
                }
                apply_command(command, &mut state);
                if capabilities_from(&state.settings).is_empty() {
                    break;
                }
            }

            let Ok(Some(track)) = library_store.track(track_id) else {
                // Track vanished mid-batch (concurrent delete or DB
                // error). Skip and move on; the next batch will reflect
                // current state.
                continue;
            };
            let absolute_path = track.location.absolute_path(&library_path);
            let now_unix = (clock)();
            // sample_rate_hz and duration are populated by the metadata
            // scan and live on TrackMetadata already — recording them
            // here lets the storage row carry the audio-stream facts
            // that were in effect when this DSP pass ran, useful for
            // future renderers cross-checking the waveform's time
            // mapping against the file the user is now seeing.
            let run_context = AnalysisRunContext {
                analyzer_version,
                sample_rate: track.metadata.sample_rate_hz.unwrap_or(0),
                duration_ms: track
                    .metadata
                    .duration
                    .and_then(|duration| u32::try_from(duration.as_millis()).ok())
                    .unwrap_or(0),
                now_unix,
            };

            match (analyzer)(&absolute_path, analysis_options) {
                Ok(analysis) => {
                    let _ = library_store.record_analysis(
                        track_id,
                        &analysis,
                        capabilities_from(&state.settings),
                        run_context,
                    );
                    state.completed = state.completed.saturating_add(1);
                }
                Err(_) => {
                    let _ = library_store.record_analysis_attempt_failure(
                        track_id,
                        capabilities_from(&state.settings),
                        AnalysisAttemptContext {
                            analyzer_version,
                            now_unix,
                        },
                    );
                    state.failed = state.failed.saturating_add(1);
                }
            }

            // Estimate remaining work from the store. Cheap because
            // tracks_needing_analysis is an indexed JOIN; we do this
            // once per track so the persistent notification text stays
            // honest.
            let remaining = library_store
                .tracks_needing_analysis(
                    capabilities_from(&state.settings),
                    analyzer_version,
                    BATCH_SIZE.saturating_mul(64),
                )
                .map(|ids| ids.len() as u32)
                .unwrap_or(0);
            (progress)(SchedulerProgress::Tick {
                completed: state.completed,
                failed: state.failed,
                remaining,
            });

            thread::sleep(INTER_TRACK_PAUSE);
        }
    }
}

struct WorkerState {
    settings: AnalysisSettings,
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

fn capabilities_from(settings: &AnalysisSettings) -> AnalysisCapabilities {
    AnalysisCapabilities {
        bpm: settings.bpm,
        key: settings.key,
        waveform: settings.waveform,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::Write,
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
            mpsc as std_mpsc,
        },
        time::{Duration, Instant},
    };

    use sustain_analysis::{AnalysisError, AnalysisOptions, TrackAnalysis};
    use sustain_domain::{
        AnalysisSettings, MusicalKey, PREVIEW_SEGMENT_COUNT, Track, TrackLocation,
        TrackRelativePath, WaveformSegment, WaveformSegments,
    };
    use sustain_library_store::{
        AnalysisCapabilities, InMemoryLibraryStore, LibraryStore, TrackId,
    };
    use tempfile::TempDir;

    use super::{
        AnalysisScheduler, AnalysisSchedulerConfig, AnalyzerFn, ProgressSink, SchedulerProgress,
        UnixClockFn,
    };

    /// Place an empty file inside `library_root` at `relative` so the
    /// scheduler's "resolve to absolute path" step succeeds. The stub
    /// analyzer does not actually read the file — it returns a canned
    /// `TrackAnalysis` — so the contents do not matter.
    fn touch_in(library_root: &std::path::Path, relative: &str) -> Track {
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
        }
    }

    fn fixed_clock(value: i64) -> UnixClockFn {
        Arc::new(move || value)
    }

    fn ok_analyzer() -> AnalyzerFn {
        Arc::new(|_path, _opts| {
            Ok(TrackAnalysis {
                bpm: Some(120.0),
                key: Some(MusicalKey::CMajor),
                beatgrid: None,
                waveform_preview: WaveformSegments {
                    segment_duration_ms: 25.0,
                    segments: vec![WaveformSegment::silent(); PREVIEW_SEGMENT_COUNT],
                },
                waveform_detail: WaveformSegments {
                    segment_duration_ms: 6.67,
                    segments: vec![WaveformSegment::silent(); 512],
                },
            })
        })
    }

    fn err_analyzer() -> AnalyzerFn {
        Arc::new(|_path, _opts| {
            Err(AnalysisError::TooShort {
                path: "stub".into(),
                samples: 0,
            })
        })
    }

    /// Channel-backed progress sink: every `SchedulerProgress` the
    /// worker emits is pushed onto a queue the test can drain.
    fn capturing_sink() -> (ProgressSink, std_mpsc::Receiver<SchedulerProgress>) {
        let (tx, rx) = std_mpsc::channel();
        let sink: ProgressSink = Arc::new(move |progress| {
            // Send failure means the receiver has been dropped — the
            // test is over; nothing useful to do.
            let _ = tx.send(progress);
        });
        (sink, rx)
    }

    /// Wait up to `timeout` for a progress event matching `predicate`,
    /// returning the matching event. Fails the test on timeout.
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

    fn capability_count(capabilities: AnalysisCapabilities) -> u32 {
        u32::from(capabilities.bpm) + u32::from(capabilities.key) + u32::from(capabilities.waveform)
    }

    #[test]
    fn scheduler_processes_pending_tracks_with_settings_enabled() {
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let track = touch_in(&library_root, "alpha.flac");

        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store.save_track(track.clone()).expect("save track");

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(AnalysisSchedulerConfig {
            analyzer: ok_analyzer(),
            progress: sink,
            clock: fixed_clock(1_700_000_000),
            library_store: store.clone(),
            initial_settings: AnalysisSettings {
                bpm: true,
                key: true,
                waveform: true,
            },
            library_path: Some(library_root),
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        });

        let tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { completed: 1, .. })
        });
        if let SchedulerProgress::Tick {
            completed, failed, ..
        } = tick
        {
            assert_eq!(completed, 1);
            assert_eq!(failed, 0);
        }

        // Track should now be ineligible for re-analysis at the same
        // analyzer_version.
        let still_pending = store
            .tracks_needing_analysis(AnalysisCapabilities::all(), 1, 100)
            .expect("query");
        assert!(still_pending.is_empty(), "track should be marked attempted");

        scheduler.shutdown();
    }

    #[test]
    fn scheduler_records_failures_without_clobbering_run() {
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let track = touch_in(&library_root, "broken.flac");

        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store.save_track(track.clone()).expect("save");

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(AnalysisSchedulerConfig {
            analyzer: err_analyzer(),
            progress: sink,
            clock: fixed_clock(2_000),
            library_store: store.clone(),
            initial_settings: AnalysisSettings {
                bpm: true,
                key: false,
                waveform: false,
            },
            library_path: Some(library_root),
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        });

        let tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { failed: 1, .. })
        });
        if let SchedulerProgress::Tick {
            completed, failed, ..
        } = tick
        {
            assert_eq!(completed, 0);
            assert_eq!(failed, 1);
        }

        // The failure attempt should be stamped so the track does not
        // re-queue at the same analyzer_version.
        let pending = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: true,
                    key: false,
                    waveform: false,
                },
                1,
                10,
            )
            .expect("query");
        assert!(pending.is_empty());

        scheduler.shutdown();
    }

    #[test]
    fn scheduler_idles_when_settings_disabled() {
        let temp = TempDir::new().expect("temp dir");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let track = touch_in(temp.path(), "alpha.flac");
        store.save_track(track).expect("save");

        // Count how often the analyzer is called — must be zero with
        // every capability off.
        let calls = Arc::new(AtomicU32::new(0));
        let counted = calls.clone();
        let analyzer: AnalyzerFn = Arc::new(move |_path, _opts| {
            counted.fetch_add(1, Ordering::SeqCst);
            ok_analyzer()(std::path::Path::new("ignored"), AnalysisOptions::default())
        });

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(AnalysisSchedulerConfig {
            analyzer,
            progress: sink,
            clock: fixed_clock(0),
            library_store: store,
            initial_settings: AnalysisSettings::default(),
            library_path: Some(temp.path().to_path_buf()),
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        });

        // First emission must be Idle (no work).
        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first progress");
        assert!(matches!(first, SchedulerProgress::Idle { .. }));

        // Give the worker a moment; it must remain idle.
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        scheduler.shutdown();
    }

    #[test]
    fn settings_change_resumes_a_blocked_worker() {
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let track = touch_in(&library_root, "alpha.flac");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store.save_track(track).expect("save");

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(AnalysisSchedulerConfig {
            analyzer: ok_analyzer(),
            progress: sink,
            clock: fixed_clock(1),
            library_store: store.clone(),
            initial_settings: AnalysisSettings::default(),
            library_path: Some(library_root),
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        });

        // Initial Idle.
        let initial = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("initial progress");
        assert!(matches!(initial, SchedulerProgress::Idle { .. }));

        // Flip waveform on; the worker must wake, process, and emit a
        // Tick.
        scheduler.update_settings(AnalysisSettings {
            bpm: false,
            key: false,
            waveform: true,
        });

        let tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { completed: 1, .. })
        });
        assert!(matches!(tick, SchedulerProgress::Tick { .. }));

        scheduler.shutdown();
    }

    #[test]
    fn toggling_capabilities_off_stops_the_running_worker() {
        // Mirror of the managed-library toggle behavior: when the user
        // un-toggles a background capability, the scheduler must stop
        // analyzing additional tracks. The in-flight DSP completes
        // naturally (we do not cancel mid-track) so this asserts the
        // bounded behavior: no more than a small number of tracks get
        // analyzed after the toggle, even though many were pending.
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        for i in 0..32 {
            let relative = format!("track_{i:02}.flac");
            let mut track = touch_in(&library_root, &relative);
            track.id = TrackId::new(i + 1).expect("non-zero");
            store.save_track(track).expect("save");
        }

        // Analyzer that artificially extends per-track work so the
        // test has a chance to send the toggle-off before the worker
        // drains the queue.
        let analyzer: AnalyzerFn = Arc::new(|_path, _opts| {
            std::thread::sleep(Duration::from_millis(80));
            ok_analyzer()(std::path::Path::new("ignored"), AnalysisOptions::default())
        });

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(AnalysisSchedulerConfig {
            analyzer,
            progress: sink,
            clock: fixed_clock(1),
            library_store: store.clone(),
            initial_settings: AnalysisSettings {
                bpm: false,
                key: false,
                waveform: true,
            },
            library_path: Some(library_root),
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        });

        // Let a couple of ticks land — proves the worker is genuinely
        // analyzing — then toggle waveform off.
        let _first_tick = wait_for(
            &rx,
            Duration::from_secs(2),
            |progress| matches!(progress, SchedulerProgress::Tick { completed, .. } if *completed >= 1),
        );
        scheduler.update_settings(AnalysisSettings::default());

        // Wait for Idle to confirm the worker observed the toggle.
        let _idle = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Idle { .. })
        });

        // Snapshot how many tracks are still pending; if the worker
        // really stopped, a generous additional wait should not move
        // the count.
        let still_pending_before = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    waveform: true,
                },
                1,
                100,
            )
            .expect("query")
            .len();
        std::thread::sleep(Duration::from_millis(400));
        let still_pending_after = store
            .tracks_needing_analysis(
                AnalysisCapabilities {
                    bpm: false,
                    key: false,
                    waveform: true,
                },
                1,
                100,
            )
            .expect("query")
            .len();
        assert_eq!(
            still_pending_before, still_pending_after,
            "worker must stop processing after capabilities go to zero"
        );
        assert!(
            still_pending_after > 0,
            "test fixture should leave un-analysed tracks; saw {still_pending_after}",
        );

        scheduler.shutdown();
    }

    #[test]
    fn shutdown_returns_after_join() {
        // Smoke test: a scheduler that never gets work should still
        // shut down promptly. Regression guard against the worker
        // blocking on recv past the shutdown nudge.
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let (sink, _rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(AnalysisSchedulerConfig {
            analyzer: ok_analyzer(),
            progress: sink,
            clock: fixed_clock(0),
            library_store: store,
            initial_settings: AnalysisSettings::default(),
            library_path: None,
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        });
        let start = Instant::now();
        scheduler.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn capability_count_is_a_small_helper() {
        // Just exercises the helper so refactors don't silently break
        // it; the public API itself doesn't expose this.
        assert_eq!(
            capability_count(AnalysisCapabilities {
                bpm: true,
                key: true,
                waveform: false
            }),
            2
        );
        assert_eq!(capability_count(AnalysisCapabilities::all()), 3);
        assert_eq!(capability_count(AnalysisCapabilities::none()), 0);
    }
}
