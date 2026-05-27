// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Paced multi-worker driver for `sustain_analysis::analyze`.
//!
//! Architecture: one supervisor thread and N worker threads. The
//! supervisor owns the command channel, the shared work queue, and
//! the lifecycle of the worker pool. Workers each apply nice +
//! ionice priorities to themselves on entry, pull `WorkItem`s off
//! the shared queue, run the DSP, write results back through the
//! library store, and report progress back to the supervisor.
//!
//! ```text
//!   ApplicationRuntime -- SchedulerCommand --> [supervisor]
//!                                                |
//!                                                | WorkItem  (mpmc)
//!                                                v
//!                                            [worker 0] [worker 1] ... [worker N-1]
//!                                                |
//!                                                | WorkOutcome (mpsc)
//!                                                v
//!                                            [supervisor]
//!                                                |
//!                                                | SchedulerProgress
//!                                                v
//!                                            ProgressSink
//! ```
//!
//! The worker count is read from
//! [`crate::priority::resolve_worker_count`] for the current
//! [`BackgroundResourceUsage`] preset. A user changing the
//! Preferences slider while the scheduler is running triggers a
//! teardown-and-respawn of the worker pool with the new count + new
//! priorities — the work queue between supervisor and workers is
//! drained first so no track is processed twice with stale settings.
//!
//! `INTER_TRACK_PAUSE` is intentionally short (25 ms). With every
//! worker running under +10 / IO best-effort-7 (`Balanced`) or +19 /
//! IO idle (`Innocuous`), the kernel scheduler is the real
//! throttling mechanism; the pause only exists to keep the worker
//! loop from spinning when the store has nothing to dispense.

use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use sustain_analysis::{AnalysisError, AnalysisOptions, TrackAnalysis};
use sustain_domain::{AnalysisSettings, BackgroundResourceUsage, TrackId};
use sustain_library_store::{AnalysisCapabilities, AnalysisContext, LibraryStore};

use crate::priority::{self, IoPriorityClass, NiceLevel};

/// How long each worker sleeps between two consecutive analyses.
/// Short — the heavy lifting (CPU/IO throttling) happens via the
/// nice and ionice priorities the worker applies to itself; the
/// pause only exists to keep the loop from yelling at the store
/// every microsecond when there is nothing to do.
const INTER_TRACK_PAUSE: Duration = Duration::from_millis(25);

/// How many tracks to fetch from the store per
/// `tracks_needing_analysis` query. Small enough that capability
/// changes propagate within a few tracks, large enough that we are
/// not querying the database between every track even on a fast
/// pool of workers.
const BATCH_SIZE: usize = 16;

/// The DSP function the worker calls per track. Production composes a
/// [`sustain_analysis::Analyzer`] and only calls the band methods for
/// capabilities that are currently active, so a track scheduled with
/// `bpm: true, key: false, waveform: false` does **not** pay for the
/// chroma extraction or full-track waveform decode. Tests substitute
/// a closure that returns canned `TrackAnalysis` values and may
/// ignore the capability mask.
pub type AnalyzerFn = Arc<
    dyn Fn(&Path, AnalysisCapabilities, AnalysisOptions) -> Result<TrackAnalysis, AnalysisError>
        + Send
        + Sync,
>;

/// Sink for progress updates emitted by the supervisor thread. The
/// runtime wraps this in an `async_channel` send so notifications
/// surface on the GTK main loop without the supervisor touching
/// widgets directly.
pub type ProgressSink = Arc<dyn Fn(SchedulerProgress) + Send + Sync>;

/// Sink invoked once per track after a successful `record_analysis`
/// landed BPM/key/waveform data into the store. The runtime wraps
/// this in an `async_channel` send so the UI shell refreshes the
/// matching row on the main loop. Stays a no-op when no sink is
/// installed (tests, headless deployments).
pub type TrackUpdatedSink = Arc<dyn Fn(TrackId) + Send + Sync>;

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
    /// The pool has caught up: either the queue is empty or every
    /// requested capability is currently disabled. UI side: dismiss
    /// the persistent "analysing" notification, emit an ephemeral
    /// summary if `completed + failed > 0`.
    Idle { completed: u32, failed: u32 },
}

/// Bundle of dependencies the scheduler captures at start-up. Grouped
/// so the supervisor signature stays manageable and so tests can
/// build a config struct once and reuse it across cases.
pub struct AnalysisSchedulerConfig {
    pub analyzer: AnalyzerFn,
    pub progress: ProgressSink,
    /// Optional refresh signal: after every successful analysis pass
    /// the worker pushes the touched `TrackId` so the runtime can
    /// reload that row from the store into its in-memory copy. `None`
    /// when the embedder does not care about live UI refreshes.
    pub track_updated: Option<TrackUpdatedSink>,
    pub clock: UnixClockFn,
    pub library_store: Arc<dyn LibraryStore>,
    pub initial_settings: AnalysisSettings,
    pub initial_resource_usage: BackgroundResourceUsage,
    pub library_path: Option<PathBuf>,
    pub analyzer_version: u32,
    pub analysis_options: AnalysisOptions,
}

#[derive(Clone, Debug)]
enum SchedulerCommand {
    SettingsChanged(AnalysisSettings),
    ResourceUsageChanged(BackgroundResourceUsage),
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
            .name("sustain-analysis-supervisor".to_owned())
            .spawn(move || supervisor_loop(receiver, config))
            .expect("spawn analysis scheduler supervisor thread");
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

    /// Tell the supervisor that the resource-usage preset changed.
    /// The supervisor will drain the in-flight work queue, tear down
    /// the worker pool, and spin up a fresh pool sized + niced for
    /// the new preset.
    pub fn update_resource_usage(&self, usage: BackgroundResourceUsage) {
        let _ = self
            .sender
            .send(SchedulerCommand::ResourceUsageChanged(usage));
    }

    pub fn set_library_path(&self, path: Option<PathBuf>) {
        let _ = self.sender.send(SchedulerCommand::LibraryPathChanged(path));
    }

    pub fn wake(&self) {
        let _ = self.sender.send(SchedulerCommand::Wake);
    }

    /// Send Shutdown, drop the sender, and join the supervisor. Blocks
    /// until the supervisor finishes draining the queue and returns
    /// from its loop. In-flight DSP completes naturally — we do not
    /// cancel mid-track.
    pub fn shutdown(mut self) {
        let _ = self.sender.send(SchedulerCommand::Shutdown);
        // Drop the live sender so even if Shutdown was lost (impossible
        // with mpsc, but defensive) the supervisor's `recv` returns Err.
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AnalysisScheduler {
    fn drop(&mut self) {
        // Best-effort cleanup if `shutdown` was not called. Don't
        // block on join in Drop — Drop may run on the GTK main thread.
        let _ = self.sender.send(SchedulerCommand::Shutdown);
        let (placeholder, _) = mpsc::channel();
        let _ = std::mem::replace(&mut self.sender, placeholder);
    }
}

/// Per-track work unit handed to workers via the shared MPMC queue.
struct WorkItem {
    track_id: TrackId,
    absolute_path: PathBuf,
    capabilities: AnalysisCapabilities,
    context: AnalysisContext,
}

/// Per-track outcome workers report back to the supervisor. The
/// `track_id` is the same id the supervisor dispatched; the
/// supervisor needs it to remove the entry from its in-flight set
/// (the set is what stops a track from being re-dispatched while a
/// worker is still chewing on it).
struct WorkOutcome {
    track_id: TrackId,
    succeeded: bool,
}

/// Shared dependencies every worker captures so it can run the DSP
/// + persist + report progress without going back to the supervisor.
struct WorkerCtx {
    work_rx: async_channel::Receiver<WorkItem>,
    outcome_tx: mpsc::Sender<WorkOutcome>,
    library_store: Arc<dyn LibraryStore>,
    analyzer: AnalyzerFn,
    track_updated: Option<TrackUpdatedSink>,
    analysis_options: AnalysisOptions,
    priority_pair: (NiceLevel, IoPriorityClass),
}

/// Handles to the running pool, kept by the supervisor for clean
/// teardown across resource-usage changes.
struct WorkerPool {
    work_tx: async_channel::Sender<WorkItem>,
    handles: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    fn spawn(
        worker_count: usize,
        usage: BackgroundResourceUsage,
        library_store: Arc<dyn LibraryStore>,
        analyzer: AnalyzerFn,
        track_updated: Option<TrackUpdatedSink>,
        analysis_options: AnalysisOptions,
        outcome_tx: mpsc::Sender<WorkOutcome>,
    ) -> Self {
        // Bounded queue: keep it small so a teardown-and-respawn does
        // not have a huge backlog of pre-dispatched work to drain. One
        // item per worker plus a one-item slack is plenty — the
        // supervisor's per-track dispatch loop refills as workers free
        // up.
        let queue_capacity = worker_count.max(1).saturating_add(1);
        let (work_tx, work_rx) = async_channel::bounded::<WorkItem>(queue_capacity);
        let priority_pair = priority::priority_for(usage);
        let mut handles = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let ctx = WorkerCtx {
                work_rx: work_rx.clone(),
                outcome_tx: outcome_tx.clone(),
                library_store: library_store.clone(),
                analyzer: analyzer.clone(),
                track_updated: track_updated.clone(),
                analysis_options,
                priority_pair,
            };
            let handle = thread::Builder::new()
                .name(format!("sustain-analysis-worker-{index}"))
                .spawn(move || worker_loop(ctx))
                .expect("spawn analysis worker thread");
            handles.push(handle);
        }
        Self { work_tx, handles }
    }

    /// Close the queue (signals workers to exit once it drains) and
    /// join them. Blocks until every worker has returned.
    fn shutdown(self) {
        // Closing the sender side prevents the supervisor from
        // accidentally pushing more work, and once the receivers drain
        // they observe a closed channel and exit.
        self.work_tx.close();
        for handle in self.handles {
            let _ = handle.join();
        }
    }
}

fn worker_loop(ctx: WorkerCtx) {
    // Apply the resource-usage preset to *this* thread. A failure here
    // is non-fatal: the worker still runs at default priority, which
    // is the same fall-back the kernel produces if we did not call at
    // all. We do not log because the scheduler runs on every launch
    // and a sandbox without the right caps would spam the journal.
    let _ = priority::apply_to_current_thread(ctx.priority_pair.0, ctx.priority_pair.1);

    loop {
        let item = match ctx.work_rx.recv_blocking() {
            Ok(item) => item,
            // Sender closed: pool is being torn down. Exit cleanly.
            Err(_) => return,
        };

        let succeeded =
            match (ctx.analyzer)(&item.absolute_path, item.capabilities, ctx.analysis_options) {
                Ok(analysis) => {
                    let _ = ctx.library_store.record_analysis(
                        item.track_id,
                        &analysis,
                        item.capabilities,
                        item.context,
                    );
                    if let Some(notify) = ctx.track_updated.as_deref() {
                        notify(item.track_id);
                    }
                    true
                }
                Err(_) => {
                    let _ = ctx.library_store.record_analysis_attempt_failure(
                        item.track_id,
                        item.capabilities,
                        item.context,
                    );
                    false
                }
            };
        let outcome = WorkOutcome {
            track_id: item.track_id,
            succeeded,
        };

        // Outcome channel send only fails on a closed receiver, which
        // happens during shutdown; nothing useful to do in that case.
        let _ = ctx.outcome_tx.send(outcome);

        thread::sleep(INTER_TRACK_PAUSE);
    }
}

struct SupervisorState {
    settings: AnalysisSettings,
    resource_usage: BackgroundResourceUsage,
    library_path: Option<PathBuf>,
    completed: u32,
    failed: u32,
}

fn supervisor_loop(receiver: mpsc::Receiver<SchedulerCommand>, config: AnalysisSchedulerConfig) {
    let AnalysisSchedulerConfig {
        analyzer,
        progress,
        track_updated,
        clock,
        library_store,
        initial_settings,
        initial_resource_usage,
        library_path,
        analyzer_version,
        analysis_options,
    } = config;

    let mut state = SupervisorState {
        settings: initial_settings,
        resource_usage: initial_resource_usage,
        library_path,
        completed: 0,
        failed: 0,
    };

    let (outcome_tx, outcome_rx) = mpsc::channel::<WorkOutcome>();
    let mut pool: Option<WorkerPool> = None;
    // Streaming dispatcher state.
    //
    // `in_flight` holds the ids of every track the supervisor has
    // handed to the worker pool but not yet seen an outcome for.
    // Those tracks are also still reported by
    // `tracks_needing_analysis` (their `record_analysis` hasn't
    // landed yet), so the set doubles as the dedup filter against
    // the store's query results.
    //
    // `pending` holds ids pulled from the store but not yet
    // shipped to a worker. The dispatch loop drains it into the
    // bounded async-channel via `try_send`, which means the
    // supervisor never blocks waiting for queue space — once the
    // channel is full, it stays at the head of `pending` and the
    // outer loop re-enters via the outcome wait below.
    let mut in_flight: HashSet<TrackId> = HashSet::new();
    let mut pending: VecDeque<TrackId> = VecDeque::new();

    loop {
        // 1. Drain commands. A resource-usage flip tears down the
        //    pool here; we detect that by comparing the
        //    before/after Option and clear the streaming
        //    bookkeeping so the next iteration starts fresh.
        let pool_was_alive = pool.is_some();
        match drain_commands(&receiver, &mut state, &mut pool) {
            DrainOutcome::Shutdown => {
                if let Some(p) = pool.take() {
                    p.shutdown();
                }
                drain_outcomes_blocking_until_quiet(&outcome_rx, &mut in_flight);
                return;
            }
            DrainOutcome::Continue => {}
        }
        if pool_was_alive && pool.is_none() {
            // A command (resource-usage flip) tore the pool down.
            // The old pool's last-second outcomes may still be in
            // transit; we discard the in-flight set so the new
            // pool starts clean, and `apply_outcome`'s
            // `HashSet::remove` is defensive for the stragglers.
            in_flight.clear();
            pending.clear();
        }

        let capabilities = capabilities_from(&state.settings);
        let library_path = match state.library_path.clone() {
            Some(path) if !capabilities.is_empty() => path,
            _ => {
                // Disabled or no library path. Drain outcomes so
                // running totals stay honest, then either go idle
                // (nothing in flight) or wait for the tail.
                drain_outcomes_nonblocking(
                    &outcome_rx,
                    &mut state,
                    &mut in_flight,
                    &library_store,
                    &progress,
                    analyzer_version,
                    capabilities,
                );
                if in_flight.is_empty() {
                    pending.clear();
                    if let Some(p) = pool.take() {
                        p.shutdown();
                    }
                    emit_idle(&progress, &mut state);
                    if !block_for_next_command(&receiver, &mut state, &mut pool) {
                        if let Some(p) = pool.take() {
                            p.shutdown();
                        }
                        drain_outcomes_blocking_until_quiet(&outcome_rx, &mut in_flight);
                        return;
                    }
                } else {
                    wait_for_outcome_with_timeout(
                        &outcome_rx,
                        &mut state,
                        &mut in_flight,
                        &library_store,
                        &progress,
                        analyzer_version,
                        capabilities,
                    );
                }
                continue;
            }
        };

        // 2. Make sure a pool is alive and sized to the current preset.
        if pool.is_none() {
            pool = Some(spawn_pool(
                state.resource_usage,
                &library_store,
                &analyzer,
                &track_updated,
                analysis_options,
                &outcome_tx,
            ));
        }
        let work_tx = pool
            .as_ref()
            .expect("pool was just ensured")
            .work_tx
            .clone();

        // 3. Refill `pending` from the store when the local
        //    buffer empties. We request enough rows to cover
        //    the in-flight set (which the store still returns
        //    as "needs analysis") plus one batch of new work,
        //    then drop the overlap via the HashSet filter.
        //
        //    The query happens on the cadence of "buffer empty"
        //    rather than "in-flight zero", so workers stay fed
        //    while the previous batch's long tail finishes —
        //    that's the fix for the multi-second idle gaps
        //    between batches that the old "wait for in_flight==0"
        //    gate produced.
        if pending.is_empty() {
            let limit = BATCH_SIZE.saturating_add(in_flight.len());
            match library_store.tracks_needing_analysis(capabilities, analyzer_version, limit) {
                Ok(fresh) => {
                    for id in fresh {
                        if !in_flight.contains(&id) {
                            pending.push_back(id);
                        }
                    }
                }
                Err(_) => {
                    // A store error here would be alarming; tear the
                    // pool down and wait for an explicit nudge so we
                    // do not hot-loop on a broken database.
                    if let Some(p) = pool.take() {
                        p.shutdown();
                    }
                    if !block_for_next_command(&receiver, &mut state, &mut pool) {
                        return;
                    }
                    continue;
                }
            }

            if pending.is_empty() && in_flight.is_empty() {
                // Truly caught up: nothing pending, nothing in flight.
                if let Some(p) = pool.take() {
                    p.shutdown();
                }
                emit_idle(&progress, &mut state);
                if !block_for_next_command(&receiver, &mut state, &mut pool) {
                    return;
                }
                continue;
            }
        }

        // 4. Dispatch from `pending` until either the channel is
        //    full (workers saturated) or `pending` is empty.
        //    `try_send` keeps the supervisor responsive to
        //    commands; anything that didn't fit stays at the head
        //    of `pending` for the next iteration.
        while let Some(&track_id) = pending.front() {
            // Re-check commands between dispatch attempts so a
            // capability toggle or resource-usage flip mid-batch
            // applies promptly. `command_drain_step` returns false
            // on Shutdown.
            if !command_drain_step(&receiver, &mut state, &mut pool) {
                if let Some(p) = pool.take() {
                    p.shutdown();
                }
                drain_outcomes_blocking_until_quiet(&outcome_rx, &mut in_flight);
                return;
            }
            let live_caps = capabilities_from(&state.settings);
            if live_caps.is_empty() {
                // Capabilities went to zero mid-dispatch; leave
                // `pending` untouched and let the next outer
                // iteration drop the pool + emit idle.
                break;
            }
            if pool.is_none() {
                // Resource-usage flip tore the pool down between
                // the work_tx clone above and this iteration.
                break;
            }

            let Ok(Some(track)) = library_store.track(track_id) else {
                // Track vanished (concurrent delete or DB error).
                pending.pop_front();
                continue;
            };
            let absolute_path = track.location.absolute_path(&library_path);
            let context = AnalysisContext {
                analyzer_version,
                now_unix: (clock)(),
            };
            let item = WorkItem {
                track_id,
                absolute_path,
                capabilities: live_caps,
                context,
            };
            match work_tx.try_send(item) {
                Ok(()) => {
                    pending.pop_front();
                    in_flight.insert(track_id);
                }
                Err(async_channel::TrySendError::Full(_)) => {
                    // Workers saturated; the outcome wait below
                    // will free space when one finishes.
                    break;
                }
                Err(async_channel::TrySendError::Closed(_)) => {
                    // Pool was torn down. Next iteration respawns.
                    break;
                }
            }
        }

        // 5. Wait for an outcome (short timeout for command
        //    responsiveness). Outcomes are the dispatcher's clock:
        //    each one frees a queue slot, so the next iteration
        //    can dispatch more from `pending`.
        if in_flight.is_empty() && pending.is_empty() {
            // Nothing to wait on and nothing to dispatch — block
            // on the command channel so we don't busy-loop.
            if !block_for_next_command(&receiver, &mut state, &mut pool) {
                return;
            }
        } else {
            wait_for_outcome_with_timeout(
                &outcome_rx,
                &mut state,
                &mut in_flight,
                &library_store,
                &progress,
                analyzer_version,
                capabilities,
            );
        }
    }
}

/// Wait up to a short timeout for one outcome, fold it into the
/// running totals, and emit a Tick. The timeout keeps the supervisor
/// responsive to commands even when workers are mid-DSP.
fn wait_for_outcome_with_timeout(
    outcome_rx: &mpsc::Receiver<WorkOutcome>,
    state: &mut SupervisorState,
    in_flight: &mut HashSet<TrackId>,
    library_store: &Arc<dyn LibraryStore>,
    progress: &ProgressSink,
    analyzer_version: u32,
    capabilities: AnalysisCapabilities,
) {
    match outcome_rx.recv_timeout(Duration::from_millis(50)) {
        Ok(outcome) => {
            apply_outcome(
                outcome,
                state,
                in_flight,
                library_store,
                progress,
                analyzer_version,
                capabilities,
            );
        }
        Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
    }
}

/// Block on the outcome channel until every in-flight item has been
/// acknowledged. Called during shutdown so the pool's `join` does not
/// race against in-flight tracks that haven't reported back yet.
fn drain_outcomes_blocking_until_quiet(
    outcome_rx: &mpsc::Receiver<WorkOutcome>,
    in_flight: &mut HashSet<TrackId>,
) {
    while !in_flight.is_empty() {
        match outcome_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(outcome) => {
                in_flight.remove(&outcome.track_id);
            }
            Err(_) => {
                // Either disconnect or timeout — either way the
                // workers won't be sending more outcomes. Give up
                // tracking; the pool join below will block on the
                // worker handles anyway.
                break;
            }
        }
    }
}

fn apply_outcome(
    outcome: WorkOutcome,
    state: &mut SupervisorState,
    in_flight: &mut HashSet<TrackId>,
    library_store: &Arc<dyn LibraryStore>,
    progress: &ProgressSink,
    analyzer_version: u32,
    capabilities: AnalysisCapabilities,
) {
    if outcome.succeeded {
        state.completed = state.completed.saturating_add(1);
    } else {
        state.failed = state.failed.saturating_add(1);
    }
    // The HashSet::remove is harmless when the id isn't there — which
    // happens after a resource-usage flip drops the in-flight set
    // (the old pool's last few outcomes can still trickle in).
    in_flight.remove(&outcome.track_id);
    let remaining = library_store
        .tracks_needing_analysis(
            capabilities,
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
}

/// Pop every queued outcome without blocking, fold it into the running
/// totals, and emit a Tick per outcome so the UI's running count moves
/// in real time. Cheap because the outcomes are small structs.
fn drain_outcomes_nonblocking(
    outcome_rx: &mpsc::Receiver<WorkOutcome>,
    state: &mut SupervisorState,
    in_flight: &mut HashSet<TrackId>,
    library_store: &Arc<dyn LibraryStore>,
    progress: &ProgressSink,
    analyzer_version: u32,
    capabilities: AnalysisCapabilities,
) {
    while let Ok(outcome) = outcome_rx.try_recv() {
        apply_outcome(
            outcome,
            state,
            in_flight,
            library_store,
            progress,
            analyzer_version,
            capabilities,
        );
    }
}

fn emit_idle(progress: &ProgressSink, state: &mut SupervisorState) {
    (progress)(SchedulerProgress::Idle {
        completed: state.completed,
        failed: state.failed,
    });
    state.completed = 0;
    state.failed = 0;
}

/// Wait on the next command, then apply it. Returns `false` if
/// shutdown was requested.
fn block_for_next_command(
    receiver: &mpsc::Receiver<SchedulerCommand>,
    state: &mut SupervisorState,
    pool: &mut Option<WorkerPool>,
) -> bool {
    match receiver.recv() {
        Ok(SchedulerCommand::Shutdown) | Err(_) => false,
        Ok(command) => {
            apply_command(command, state, pool);
            true
        }
    }
}

/// Drain a single pending command (non-blocking). Returns false on
/// shutdown (the supervisor must return to the caller immediately).
fn command_drain_step(
    receiver: &mpsc::Receiver<SchedulerCommand>,
    state: &mut SupervisorState,
    pool: &mut Option<WorkerPool>,
) -> bool {
    if let Some(command) = receiver.try_iter().next() {
        if matches!(command, SchedulerCommand::Shutdown) {
            if let Some(p) = pool.take() {
                p.shutdown();
            }
            return false;
        }
        apply_command(command, state, pool);
    }
    true
}

enum DrainOutcome {
    Continue,
    Shutdown,
}

fn drain_commands(
    receiver: &mpsc::Receiver<SchedulerCommand>,
    state: &mut SupervisorState,
    pool: &mut Option<WorkerPool>,
) -> DrainOutcome {
    while let Ok(command) = receiver.try_recv() {
        if matches!(command, SchedulerCommand::Shutdown) {
            return DrainOutcome::Shutdown;
        }
        apply_command(command, state, pool);
    }
    DrainOutcome::Continue
}

fn apply_command(
    command: SchedulerCommand,
    state: &mut SupervisorState,
    pool: &mut Option<WorkerPool>,
) {
    match command {
        SchedulerCommand::SettingsChanged(settings) => {
            state.settings = settings;
        }
        SchedulerCommand::ResourceUsageChanged(usage) => {
            if state.resource_usage != usage {
                state.resource_usage = usage;
                // Force a respawn at the new size + new priority. The
                // dispatch loop will recreate the pool on next iteration.
                if let Some(p) = pool.take() {
                    p.shutdown();
                }
            }
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

fn spawn_pool(
    usage: BackgroundResourceUsage,
    library_store: &Arc<dyn LibraryStore>,
    analyzer: &AnalyzerFn,
    track_updated: &Option<TrackUpdatedSink>,
    analysis_options: AnalysisOptions,
    outcome_tx: &mpsc::Sender<WorkOutcome>,
) -> WorkerPool {
    let worker_count = priority::resolve_worker_count(usage);
    WorkerPool::spawn(
        worker_count,
        usage,
        library_store.clone(),
        analyzer.clone(),
        track_updated.clone(),
        analysis_options,
        outcome_tx.clone(),
    )
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
        AnalysisSettings, BackgroundResourceUsage, MusicalKey, PREVIEW_SEGMENT_COUNT, Track,
        TrackLocation, TrackRelativePath, WaveformSegment, WaveformSegments,
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
            has_embedded_artwork: None,
        }
    }

    fn fixed_clock(value: i64) -> UnixClockFn {
        Arc::new(move || value)
    }

    fn ok_analyzer() -> AnalyzerFn {
        Arc::new(|_path, _caps, _opts| {
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
        Arc::new(|_path, _caps, _opts| {
            Err(AnalysisError::TooShort {
                path: "stub".into(),
                samples: 0,
            })
        })
    }

    /// Channel-backed progress sink: every `SchedulerProgress` the
    /// supervisor emits is pushed onto a queue the test can drain.
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

    fn deterministic_config(
        analyzer: AnalyzerFn,
        progress: ProgressSink,
        clock: UnixClockFn,
        library_store: Arc<dyn LibraryStore>,
        settings: AnalysisSettings,
        library_path: Option<std::path::PathBuf>,
    ) -> AnalysisSchedulerConfig {
        AnalysisSchedulerConfig {
            analyzer,
            progress,
            track_updated: None,
            clock,
            library_store,
            initial_settings: settings,
            // Tests run with a single worker so progress events arrive
            // in a predictable order regardless of host core count.
            initial_resource_usage: BackgroundResourceUsage::Innocuous,
            library_path,
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        }
    }

    #[test]
    fn scheduler_processes_pending_tracks_with_settings_enabled() {
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let track = touch_in(&library_root, "alpha.flac");

        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store.save_track(track.clone()).expect("save track");

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            ok_analyzer(),
            sink,
            fixed_clock(1_700_000_000),
            store.clone(),
            AnalysisSettings {
                bpm: true,
                key: true,
                waveform: true,
            },
            Some(library_root),
        ));

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
        let scheduler = AnalysisScheduler::start(deterministic_config(
            err_analyzer(),
            sink,
            fixed_clock(2_000),
            store.clone(),
            AnalysisSettings {
                bpm: true,
                key: false,
                waveform: false,
            },
            Some(library_root),
        ));

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
        let analyzer: AnalyzerFn = Arc::new(move |_path, _caps, _opts| {
            counted.fetch_add(1, Ordering::SeqCst);
            ok_analyzer()(
                std::path::Path::new("ignored"),
                AnalysisCapabilities::all(),
                AnalysisOptions::default(),
            )
        });

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            analyzer,
            sink,
            fixed_clock(0),
            store,
            AnalysisSettings::default(),
            Some(temp.path().to_path_buf()),
        ));

        // First emission must be Idle (no work).
        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first progress");
        assert!(matches!(first, SchedulerProgress::Idle { .. }));

        // Give the supervisor a moment; it must remain idle.
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        scheduler.shutdown();
    }

    #[test]
    fn settings_change_resumes_a_blocked_scheduler() {
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let track = touch_in(&library_root, "alpha.flac");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store.save_track(track).expect("save");

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            ok_analyzer(),
            sink,
            fixed_clock(1),
            store.clone(),
            AnalysisSettings::default(),
            Some(library_root),
        ));

        // Initial Idle.
        let initial = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("initial progress");
        assert!(matches!(initial, SchedulerProgress::Idle { .. }));

        // Flip waveform on; the supervisor must wake, process, and
        // emit a Tick.
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
    fn toggling_capabilities_off_stops_the_running_scheduler() {
        // When the user un-toggles a background capability the
        // scheduler must stop analyzing additional tracks. The
        // in-flight DSP completes naturally (we do not cancel
        // mid-track) so this asserts the bounded behavior: no more
        // than a small number of tracks get analyzed after the
        // toggle, even though many were pending.
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
        // test has a chance to send the toggle-off before the
        // scheduler drains the queue.
        let analyzer: AnalyzerFn = Arc::new(|_path, _caps, _opts| {
            std::thread::sleep(Duration::from_millis(80));
            ok_analyzer()(
                std::path::Path::new("ignored"),
                AnalysisCapabilities::all(),
                AnalysisOptions::default(),
            )
        });

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            analyzer,
            sink,
            fixed_clock(1),
            store.clone(),
            AnalysisSettings {
                bpm: false,
                key: false,
                waveform: true,
            },
            Some(library_root),
        ));

        // Let a couple of ticks land — proves the scheduler is
        // genuinely analyzing — then toggle waveform off.
        let _first_tick = wait_for(
            &rx,
            Duration::from_secs(2),
            |progress| matches!(progress, SchedulerProgress::Tick { completed, .. } if *completed >= 1),
        );
        scheduler.update_settings(AnalysisSettings::default());

        // Wait for Idle to confirm the supervisor observed the toggle.
        let _idle = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Idle { .. })
        });

        // Snapshot how many tracks are still pending; if the
        // scheduler really stopped, a generous additional wait should
        // not move the count.
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
            "scheduler must stop processing after capabilities go to zero"
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
        // shut down promptly. Regression guard against the supervisor
        // blocking on recv past the shutdown nudge.
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        let (sink, _rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            ok_analyzer(),
            sink,
            fixed_clock(0),
            store,
            AnalysisSettings::default(),
            None,
        ));
        let start = Instant::now();
        scheduler.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn track_updated_sink_fires_after_successful_analysis() {
        use std::sync::mpsc as std_mpsc;

        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let track = touch_in(&library_root, "alpha.flac");

        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store.save_track(track.clone()).expect("save track");

        let (notify_tx, notify_rx) = std_mpsc::channel::<sustain_domain::TrackId>();
        let track_updated: super::TrackUpdatedSink = Arc::new(move |id| {
            let _ = notify_tx.send(id);
        });

        let (sink, _rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(AnalysisSchedulerConfig {
            analyzer: ok_analyzer(),
            progress: sink,
            track_updated: Some(track_updated),
            clock: fixed_clock(1),
            library_store: store,
            initial_settings: AnalysisSettings {
                bpm: true,
                key: true,
                waveform: true,
            },
            initial_resource_usage: BackgroundResourceUsage::Innocuous,
            library_path: Some(library_root),
            analyzer_version: 1,
            analysis_options: AnalysisOptions::default(),
        });

        let observed = notify_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("track_updated sink fires once analysis completes");
        assert_eq!(observed, track.id);

        scheduler.shutdown();
    }

    #[test]
    fn resource_usage_change_respawns_pool_without_losing_work() {
        // Pool teardown + respawn must not drop pending tracks: after
        // the resource-usage flip, the scheduler should continue
        // draining the queue under the new preset.
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        for i in 0..8 {
            let relative = format!("track_{i:02}.flac");
            let mut track = touch_in(&library_root, &relative);
            track.id = TrackId::new(i + 1).expect("non-zero");
            store.save_track(track).expect("save");
        }

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            ok_analyzer(),
            sink,
            fixed_clock(1),
            store.clone(),
            AnalysisSettings {
                bpm: true,
                key: true,
                waveform: true,
            },
            Some(library_root),
        ));

        // Let the first track land so we know the pool is alive.
        let _first = wait_for(
            &rx,
            Duration::from_secs(2),
            |progress| matches!(progress, SchedulerProgress::Tick { completed, .. } if *completed >= 1),
        );

        scheduler.update_resource_usage(BackgroundResourceUsage::Aggressive);

        // After respawn the remaining tracks should still drain.
        let _idle = wait_for(&rx, Duration::from_secs(5), |progress| {
            matches!(progress, SchedulerProgress::Idle { .. })
        });

        let still_pending = store
            .tracks_needing_analysis(AnalysisCapabilities::all(), 1, 100)
            .expect("query");
        assert!(
            still_pending.is_empty(),
            "respawn must not drop pending tracks; {} still pending",
            still_pending.len(),
        );

        scheduler.shutdown();
    }

    #[test]
    fn analyzer_receives_the_live_capability_mask() {
        // The scheduler must hand the analyzer the exact capability
        // set the user has on right now — not the full mask. This is
        // the integration end of "capability-gated `Analyzer`
        // returning None for the bands the caller did not ask for":
        // production wires the analyzer to only call the band methods
        // that the mask permits, so confirming the mask makes it
        // through guards against the scheduler quietly defaulting to
        // `AnalysisCapabilities::all()`.
        use std::sync::Mutex;

        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let track = touch_in(&library_root, "alpha.flac");
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());
        store.save_track(track.clone()).expect("save track");

        let observed: Arc<Mutex<Option<AnalysisCapabilities>>> = Arc::new(Mutex::new(None));
        let observed_for_closure = observed.clone();
        let analyzer: AnalyzerFn = Arc::new(move |_path, caps, _opts| {
            if let Ok(mut guard) = observed_for_closure.lock() {
                *guard = Some(caps);
            }
            ok_analyzer()(
                std::path::Path::new("ignored"),
                AnalysisCapabilities::all(),
                AnalysisOptions::default(),
            )
        });

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            analyzer,
            sink,
            fixed_clock(1),
            store,
            AnalysisSettings {
                bpm: true,
                key: false,
                waveform: false,
            },
            Some(library_root),
        ));

        let _tick = wait_for(&rx, Duration::from_secs(2), |progress| {
            matches!(progress, SchedulerProgress::Tick { completed: 1, .. })
        });

        let seen = observed
            .lock()
            .expect("lock")
            .take()
            .expect("analyzer was called");
        assert!(seen.bpm, "bpm must be set");
        assert!(!seen.key, "key must be cleared — user toggled it off");
        assert!(
            !seen.waveform,
            "waveform must be cleared — user toggled it off"
        );

        scheduler.shutdown();
    }

    #[test]
    fn streaming_dispatch_keeps_workers_fed_across_batch_boundary() {
        // Regression guard for the "6-second idle gap between
        // batches" we saw under the old supervisor: it waited for
        // `in_flight == 0` before re-querying the store, so the
        // tail end of every BATCH_SIZE-sized burst left N-2
        // workers spinning. The streaming dispatcher must keep
        // dispatching across the boundary — concretely, on a pool
        // with capacity > BATCH_SIZE, we should never see all
        // workers go simultaneously idle until the whole library
        // is done.
        //
        // We simulate that by pre-populating BATCH_SIZE*3 tracks
        // and verifying that the analyzer call rate stays smooth
        // through the run, i.e. there is never a window during
        // which `analyzer_call_count` plateaus for more than the
        // per-track slowdown the stub introduces.
        let temp = TempDir::new().expect("temp dir");
        let library_root = temp.path().to_path_buf();
        let store: Arc<dyn LibraryStore> = Arc::new(InMemoryLibraryStore::new());

        let track_count = (super::BATCH_SIZE as i64) * 3;
        for i in 0..track_count {
            let relative = format!("track_{i:03}.flac");
            let mut track = touch_in(&library_root, &relative);
            track.id = TrackId::new(i + 1).expect("non-zero");
            store.save_track(track).expect("save");
        }

        // Stub analyzer with a fixed per-track delay. The delay is
        // long enough that several outcomes overlap with the
        // dispatcher's outer loop, which is exactly when the old
        // gated-on-in-flight logic stalled.
        use std::sync::Mutex;
        let call_times: Arc<Mutex<Vec<Instant>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded_times = call_times.clone();
        let analyzer: AnalyzerFn = Arc::new(move |_path, _caps, _opts| {
            if let Ok(mut guard) = recorded_times.lock() {
                guard.push(Instant::now());
            }
            std::thread::sleep(Duration::from_millis(40));
            ok_analyzer()(
                std::path::Path::new("ignored"),
                AnalysisCapabilities::all(),
                AnalysisOptions::default(),
            )
        });

        let (sink, rx) = capturing_sink();
        let scheduler = AnalysisScheduler::start(deterministic_config(
            analyzer,
            sink,
            fixed_clock(1),
            store.clone(),
            AnalysisSettings {
                bpm: true,
                key: true,
                waveform: true,
            },
            Some(library_root),
        ));

        // Drain to Idle so we know every track was processed.
        let _ = wait_for(
            &rx,
            Duration::from_secs(30),
            |progress| matches!(progress, SchedulerProgress::Idle { completed, .. } if *completed as i64 == track_count),
        );
        scheduler.shutdown();

        // Inspect the dispatch cadence: pick the largest gap
        // between consecutive analyzer entries; on the old
        // gated-batched supervisor this gap was the batch tail
        // (≈ inter-track-sleep * inflight_count). On the
        // streaming dispatcher it should be at most one
        // inter-track sleep plus a touch of supervisor overhead.
        // We assert "no gap larger than 5x the per-track sleep",
        // which is a conservative regression bound — the old
        // behavior under a BATCH_SIZE=16 / 1-worker (Innocuous)
        // run produced gaps proportional to the whole batch.
        let times = call_times.lock().expect("lock");
        assert_eq!(
            times.len(),
            track_count as usize,
            "analyzer should be called once per track",
        );
        let mut max_gap = Duration::ZERO;
        for window in times.windows(2) {
            let gap = window[1].duration_since(window[0]);
            if gap > max_gap {
                max_gap = gap;
            }
        }
        let bound = Duration::from_millis(40 * 5);
        assert!(
            max_gap <= bound,
            "dispatch cadence stalled: max gap {:?} exceeded {:?}",
            max_gap,
            bound,
        );
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
