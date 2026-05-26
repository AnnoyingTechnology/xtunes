// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{rc::Rc, sync::mpsc, time::Duration};

use gtk::glib;

use super::{
    ApplicationRuntimeError, LibraryManagementMode, SharedRuntime, run_library_consolidation_task,
    status_bar::StatusBar,
};

pub(crate) type LibraryConsolidationRequestedCallback =
    Rc<dyn Fn(ConsolidationTrigger) -> Result<(), ApplicationRuntimeError>>;

/// Tells the consolidation pipeline who asked for the run. The status
/// bar uses this to decide whether a "0 moved, 0 missing" outcome is
/// worth surfacing: the auto-resume on every launch is silent on a
/// pure no-op (it ran, it found nothing, the user did not ask), but a
/// user-triggered run from preferences always reports its outcome so
/// the user has feedback that the click was honoured.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ConsolidationTrigger {
    UserAction,
    AutoResume,
}

/// Kicks off a consolidation pass if the user has opted into managed
/// library organization. Used at application startup so an interrupted
/// previous run (kill, crash, system power loss) resumes silently
/// instead of leaving the library half-organized forever.
///
/// Idempotent and cheap when there is nothing to do: the consolidation
/// planner returns an empty plan for an already-organized library and
/// completes immediately. Failures surface through the status bar
/// exactly like a user-triggered run; there is nothing meaningful to
/// do here on error beyond letting the consolidation callback report
/// it.
pub(crate) fn maybe_auto_resume_library_consolidation(
    runtime: &SharedRuntime,
    consolidation_requested: &LibraryConsolidationRequestedCallback,
) {
    let should_resume = {
        let runtime = runtime.borrow();
        let settings = runtime.settings();
        settings.library.management_mode == LibraryManagementMode::CopyAddedFilesIntoLibrary
            && settings.library_path().is_some_and(|path| path.is_dir())
            && !runtime.background_task_status().is_running()
    };
    if should_resume {
        let _ = consolidation_requested(ConsolidationTrigger::AutoResume);
    }
}

pub(crate) fn library_consolidation_requested_callback(
    runtime: &SharedRuntime,
    status_bar: &StatusBar,
) -> LibraryConsolidationRequestedCallback {
    let runtime = runtime.clone();
    let status_bar = status_bar.clone();

    Rc::new(move |trigger| {
        let task = {
            let mut runtime = runtime.borrow_mut();
            let task = runtime.prepare_library_consolidation()?;
            status_bar.update_task(
                runtime.background_task_status(),
                runtime.background_task_cancellation_requested(),
            );
            task
        };

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _sent = tx.send(run_library_consolidation_task(task));
        });

        poll_library_consolidation(rx, runtime.clone(), status_bar.clone(), trigger);
        Ok(())
    })
}

// Consolidation only mutates each moved track's stored relative path
// (in SQLite and in the in-memory library_tracks vec). It does not
// add, remove, or otherwise change anything the user sees:
//   - track metadata, rating, statistics, availability flag: unchanged
//   - playlist membership (linked by TrackId, not path): unchanged
//   - sidebar entries, now-playing tile, table rows: all unchanged
// The status-bar progress + completion message is the entire UI
// contract. Triggering `library_changed()` here would force the songs
// table, albums view, sidebar tree, and playlists table to rebuild
// for nothing — measured at multiple seconds of `replace_rows` work
// on a 10k library.
fn poll_library_consolidation(
    rx: mpsc::Receiver<Result<super::LibraryConsolidationResult, ApplicationRuntimeError>>,
    runtime: SharedRuntime,
    status_bar: StatusBar,
    trigger: ConsolidationTrigger,
) {
    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(Ok(result)) => {
            // An auto-resume that found nothing to move and nothing
            // missing is the boring-success case we want to disappear:
            // the user did not ask for this run and the spinner that
            // briefly showed "Organizing library…" is feedback enough.
            // Cancelled runs, failed runs, and runs that surfaced
            // missing files are kept on screen — they carry
            // information the user may want to act on.
            let dismiss_completion = matches!(trigger, ConsolidationTrigger::AutoResume)
                && result.summary.moved_tracks == 0
                && result.summary.missing_tracks == 0
                && !result.summary.cancelled;
            {
                let mut runtime = runtime.borrow_mut();
                runtime.apply_library_consolidation_result(result);
                if dismiss_completion {
                    runtime.dismiss_completed_background_task_status();
                }
            }
            refresh_task_status(&runtime, &status_bar);
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            runtime.borrow_mut().fail_library_consolidation(error);
            refresh_task_status(&runtime, &status_bar);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => {
            refresh_task_status(&runtime, &status_bar);
            glib::ControlFlow::Continue
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            runtime
                .borrow_mut()
                .fail_library_consolidation(ApplicationRuntimeError::LibraryConsolidationFailed);
            refresh_task_status(&runtime, &status_bar);
            glib::ControlFlow::Break
        }
    });
}

fn refresh_task_status(runtime: &SharedRuntime, status_bar: &StatusBar) {
    let runtime = runtime.borrow();
    status_bar.update_task(
        runtime.background_task_status(),
        runtime.background_task_cancellation_requested(),
    );
}
