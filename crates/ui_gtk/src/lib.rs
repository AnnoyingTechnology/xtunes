// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{cell::RefCell, rc::Rc};

use gtk::glib;
use gtk::prelude::*;
use main_window::build_main_window;

pub use sustain_app_runtime::{
    ApplicationCommand, ApplicationQuery, ApplicationRuntime, ApplicationRuntimeError,
    BackgroundTaskStatus, LibraryConsolidationResult, LibraryConsolidationSummary,
    LibraryImportResult, LibraryImportSummary, LibraryManagementMode, LibraryScanResult,
    LibraryScanSummary, UserSettings, run_library_consolidation_task, run_library_import_task,
    run_library_scan_task,
};

mod accent;
mod albums;
mod app_css;
mod artwork_color;
mod artwork_loader;
mod command_controller;
mod content_stack;
mod date_format;
mod library_consolidation;
mod library_import;
mod library_scan;
mod main_window;
mod mode_bar;
mod now_playing;
mod playlists_header;
mod preferences;
mod shortcuts;
mod sidebar;
mod sidebar_context;
mod smart_playlist_editor;
mod status_bar;
mod titlebar;
mod track_context;
mod track_context_ops;
mod track_info;
mod track_table;
mod window_chrome;

const TITLEBAR_HEIGHT: i32 = 72;
const TITLEBAR_LEFT_PADDING: i32 = 48;
const TITLEBAR_RIGHT_PADDING: i32 = 0;
const TITLEBAR_CONTROL_HEIGHT: i32 = 42;
const MEDIA_ICON_SIZE: i32 = 32;
const MODE_BAR_HEIGHT: i32 = 34;
const MODE_BUTTON_HEIGHT: i32 = 22;
const NOW_PLAYING_HORIZONTAL_MARGIN: i32 = TITLEBAR_HEIGHT / 2;
const NOW_PLAYING_ICON_SIZE: i32 = 16;
const NOW_PLAYING_SIDE_WIDTH: i32 = 58;
const NOW_PLAYING_WIDTH: i32 = 600;
const PREFERENCES_WIDTH: i32 = 520;
const SMART_PLAYLIST_EDITOR_WIDTH: i32 = 620;
const SMART_PLAYLIST_EDITOR_HEIGHT: i32 = 360;
const RESIZE_CORNER_SIZE: i32 = 18;
const RESIZE_EDGE_THICKNESS: i32 = 6;
const SIDEBAR_DEFAULT_WIDTH: i32 = 220;
const SIDEBAR_MIN_WIDTH: i32 = 150;
const SIDEBAR_MAX_WIDTH: i32 = 300;
const STATUS_BAR_HEIGHT: i32 = 28;
const VOLUME_WIDTH: i32 = 192;
const VOLUME_MAGNET_THRESHOLD: f64 = 0.90;
const WINDOW_SHADOW_MARGIN: i32 = 24;
/// Fixed reverse-DNS id of the installed `.desktop` entry / icon theme name.
///
/// This is the value used when **looking up the application's icon** (window
/// icon, now-playing fallback image) — those lookups must match the file
/// name shipped by the Debian package, regardless of which database the
/// running instance is pointing at.
///
/// The GTK *application id* used for single-instance routing is a separate
/// value derived from the resolved database path (see
/// `sustain-app`'s `instance_lock` module). Do not reuse `APP_ID` for
/// `gtk::Application::application_id`.
const APP_ID: &str = "io.github.open_sustain.sustain";
const SONGS_VIEW: &str = "songs";
const ALBUMS_VIEW: &str = "albums";
const PLAYLISTS_VIEW: &str = "playlists";

pub(crate) type SharedRuntime = Rc<RefCell<ApplicationRuntime>>;
pub(crate) type LibraryChangedCallback = Rc<dyn Fn()>;
pub(crate) type LibraryChangedHolder = Rc<RefCell<Option<LibraryChangedCallback>>>;
pub(crate) type TrackRowChangedCallback = Rc<dyn Fn(sustain_app_runtime::TrackId)>;
pub(crate) type TrackRowChangedHolder = Rc<RefCell<Option<TrackRowChangedCallback>>>;
/// Re-sync the `is_missing` flag on every loaded row from the
/// runtime's view of the library, repaint visible status icons, and
/// leave scroll/focus/selection untouched. Fired after operations
/// the runtime can use to flip availability without rebuilding the
/// table (a lazy-detection failed play, a library-path change that
/// re-stats existing tracks). The bulk-rebuild
/// [`LibraryChangedCallback`] would also work, but it splices the
/// store and thus blows scroll position — unacceptable when the
/// table content itself has not structurally changed.
pub(crate) type AvailabilityChangedCallback = Rc<dyn Fn()>;
pub(crate) type PlaybackChangedCallback = Rc<dyn Fn()>;
pub(crate) type ShowAlbumAction = Rc<dyn Fn(sustain_app_runtime::TrackId)>;
pub(crate) type ShowAlbumHolder = Rc<RefCell<Option<ShowAlbumAction>>>;
pub(crate) type SharedMprisService = Rc<sustain_desktop::MprisService>;
pub(crate) type MprisCommandReceiver = async_channel::Receiver<sustain_desktop::MprisCommand>;
pub(crate) type MetadataWriteResultReceiver =
    async_channel::Receiver<sustain_app_runtime::MetadataWriteResult>;
pub(crate) type ArtworkFetchResultReceiver =
    async_channel::Receiver<sustain_app_runtime::ArtworkFetchResult>;

pub fn run(mut runtime: ApplicationRuntime, application_id: &str) {
    let trun = std::time::Instant::now();
    macro_rules! tlog {
        ($label:expr) => {
            eprintln!(
                "[TIMING] {:>8.1}ms run: {}",
                trun.elapsed().as_secs_f64() * 1000.0,
                $label
            );
        };
    }
    tlog!("entered");
    let app = gtk::Application::builder()
        .application_id(application_id)
        .build();
    tlog!("gtk Application built");

    // Start the async metadata writer before wrapping the runtime in a
    // shared cell, so its worker thread is up before any UI mutation
    // can submit a job. Pair it with a result sink consumed by the main
    // loop below, so per-write failures can surface in the status bar.
    tlog!("about to start metadata writer");
    if let Err(error) = runtime.start_metadata_writer() {
        eprintln!(
            "Sustain: async metadata writer could not start ({error:?}); tag writes will run on the main thread."
        );
    }
    let (write_result_tx, write_result_rx) =
        async_channel::unbounded::<sustain_app_runtime::MetadataWriteResult>();
    runtime.set_metadata_write_result_sink(write_result_tx);
    tlog!("metadata writer running");

    // Start the artwork fetcher and install its result sink. The
    // fetcher only runs when a remote metadata service was installed
    // by the app entry; otherwise this is a no-op and any
    // `FetchArtwork` command returns `ArtworkFetchingUnavailable` at
    // dispatch time. The matching receiver is wired into the main
    // window below so successful fetches drive a `SetArtwork`
    // follow-up on the GTK main thread.
    let (fetch_result_tx, fetch_result_rx) =
        async_channel::unbounded::<sustain_app_runtime::ArtworkFetchResult>();
    runtime.set_artwork_fetch_result_sink(fetch_result_tx);
    tlog!("about to start artwork fetcher");
    if let Err(error) = runtime.start_artwork_fetcher() {
        // The only legitimate failure here is "no remote metadata
        // service installed", which is a normal state for builds
        // without networking enabled. Log and continue; the click-
        // to-fetch affordance simply stays inert.
        eprintln!(
            "Sustain: remote artwork retrieval disabled ({error:?}); the missing-artwork tile will not be clickable."
        );
    }

    tlog!("artwork fetcher started");
    let runtime = Rc::new(RefCell::new(runtime));

    // Start the MPRIS server before any window is built so the bus name
    // is claimed (or refused) deterministically at startup. The inbound
    // channel carries method calls from the MPRIS worker thread to the
    // GTK main thread, where they can safely touch the runtime.
    let (mpris_command_tx, mpris_command_rx) =
        async_channel::unbounded::<sustain_desktop::MprisCommand>();
    tlog!("about to start mpris");
    let mpris_service = match start_mpris(mpris_command_tx) {
        Ok(service) => Some(Rc::new(service)),
        Err(error) => {
            eprintln!("Sustain: MPRIS (media key) integration disabled: {error}");
            None
        }
    };
    // `connect_activate` may be invoked more than once over the
    // application lifetime (e.g. a second `gtk::Application::activate`
    // call), but the inbound receiver must only be consumed once — a
    // second consumer would race for the same commands. Take it on the
    // first activation; later activations skip the setup.
    let mpris_command_rx_holder: Rc<RefCell<Option<MprisCommandReceiver>>> =
        Rc::new(RefCell::new(Some(mpris_command_rx)));
    let write_result_rx_holder: Rc<RefCell<Option<MetadataWriteResultReceiver>>> =
        Rc::new(RefCell::new(Some(write_result_rx)));
    let fetch_result_rx_holder: Rc<RefCell<Option<ArtworkFetchResultReceiver>>> =
        Rc::new(RefCell::new(Some(fetch_result_rx)));

    tlog!("mpris done; about to connect_activate");
    app.connect_activate({
        let runtime = runtime.clone();
        move |app| {
            let tact = std::time::Instant::now();
            eprintln!(
                "[TIMING]   activate: entered (run+0={:.1}ms)",
                trun.elapsed().as_secs_f64() * 1000.0
            );
            let mpris_command_rx = mpris_command_rx_holder.borrow_mut().take();
            let write_result_rx = write_result_rx_holder.borrow_mut().take();
            let fetch_result_rx = fetch_result_rx_holder.borrow_mut().take();
            let main_window = build_main_window(
                app,
                runtime.clone(),
                mpris_service.clone(),
                mpris_command_rx,
                write_result_rx,
                fetch_result_rx,
            );
            eprintln!(
                "[TIMING]   activate: build_main_window returned at {:.1}ms",
                tact.elapsed().as_secs_f64() * 1000.0
            );
            main_window.window.present();
            eprintln!(
                "[TIMING]   activate: window.present() returned at {:.1}ms",
                tact.elapsed().as_secs_f64() * 1000.0
            );
            // Fires after the main loop has finished its current dispatch
            // batch — i.e. roughly when the window has had a chance to map.
            let tact_for_idle = tact;
            gtk::glib::idle_add_local_once(move || {
                eprintln!(
                    "[TIMING]   activate: first idle reached at {:.1}ms",
                    tact_for_idle.elapsed().as_secs_f64() * 1000.0
                );
                main_window.run_deferred_startup();
            });
        }
    });

    tlog!("about to enter app.run() (gtk main loop)");
    app.run();

    // Drain pending tag writes synchronously before exiting so a rating
    // clicked moments before close is not lost. `shutdown_metadata_writer`
    // joins the worker thread; the channel sender is dropped first so the
    // worker's `recv` returns once the queue is empty.
    let mut runtime_guard = runtime.borrow_mut();
    runtime_guard.shutdown_metadata_writer();
    runtime_guard.shutdown_artwork_fetcher();
}

/// Activate the already-running Sustain primary instance that owns
/// `application_id`, then return.
///
/// Used by `sustain-app` when the per-database single-instance lock is
/// already held: instead of opening a second window, we forward an
/// `activate` to the primary so it raises/focuses its existing window. The
/// returned `ExitCode` is whatever `gtk::Application::run` reports for the
/// short-lived remote registration (`0` on success, non-zero when the
/// inter-process registration itself failed).
///
/// **Must not be called from the primary process.** The caller is expected
/// to have already determined that another instance owns the lock; calling
/// this from the primary would just register a second `gtk::Application`
/// against the same id and either dispatch activate to itself or fail to
/// register, depending on timing.
pub fn forward_activate(application_id: &str) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(application_id)
        .build();
    // The primary's connect_activate handler raises its window. Our
    // local activate signal handler is only reached if no primary
    // existed at register time — in which case we deliberately do
    // nothing (and exit), because spinning up a second main loop here
    // would defeat the single-instance guarantee.
    app.connect_activate(|_app| {});
    app.run()
}

fn start_mpris(
    command_tx: async_channel::Sender<sustain_desktop::MprisCommand>,
) -> sustain_desktop::DesktopResult<sustain_desktop::MprisService> {
    sustain_desktop::MprisService::start(sustain_desktop::MprisStartConfig {
        command_sink: sustain_desktop::MprisPlaybackSink::new(move |command| {
            // Unbounded channel: try_send only fails if closed, i.e. the
            // GTK main loop has exited and the receiver was dropped.
            // Silent drop is the right behavior at shutdown.
            let _ = command_tx.try_send(command);
        }),
    })
}
