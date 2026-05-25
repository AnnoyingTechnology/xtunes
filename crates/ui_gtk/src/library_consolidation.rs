// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{rc::Rc, sync::mpsc, time::Duration};

use gtk::glib;

use super::{
    ApplicationRuntimeError, LibraryChangedCallback, SharedRuntime, run_library_consolidation_task,
    status_bar::StatusBar,
};

pub(crate) type LibraryConsolidationRequestedCallback =
    Rc<dyn Fn() -> Result<(), ApplicationRuntimeError>>;

pub(crate) fn library_consolidation_requested_callback(
    runtime: &SharedRuntime,
    library_changed: LibraryChangedCallback,
    status_bar: &StatusBar,
) -> LibraryConsolidationRequestedCallback {
    let runtime = runtime.clone();
    let status_bar = status_bar.clone();

    Rc::new(move || {
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

        poll_library_consolidation(
            rx,
            runtime.clone(),
            library_changed.clone(),
            status_bar.clone(),
        );
        Ok(())
    })
}

fn poll_library_consolidation(
    rx: mpsc::Receiver<Result<super::LibraryConsolidationResult, ApplicationRuntimeError>>,
    runtime: SharedRuntime,
    library_changed: LibraryChangedCallback,
    status_bar: StatusBar,
) {
    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(Ok(result)) => {
            {
                runtime
                    .borrow_mut()
                    .apply_library_consolidation_result(result);
            }
            library_changed();
            refresh_task_status(&runtime, &status_bar);
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            {
                runtime.borrow_mut().fail_library_consolidation(error);
            }
            refresh_task_status(&runtime, &status_bar);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => {
            refresh_task_status(&runtime, &status_bar);
            glib::ControlFlow::Continue
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            {
                runtime.borrow_mut().fail_library_consolidation(
                    ApplicationRuntimeError::LibraryConsolidationFailed,
                );
            }
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
