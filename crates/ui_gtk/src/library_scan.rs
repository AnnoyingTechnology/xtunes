// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{path::PathBuf, rc::Rc, sync::mpsc, time::Duration};

use gtk::glib;

use super::{
    ApplicationRuntimeError, LibraryChangedCallback, SharedRuntime, run_library_scan_task,
};

pub(crate) type LibraryScanRequestedCallback =
    Rc<dyn Fn(PathBuf) -> Result<(), ApplicationRuntimeError>>;

pub(crate) fn library_scan_requested_callback(
    runtime: &SharedRuntime,
    library_changed: LibraryChangedCallback,
) -> LibraryScanRequestedCallback {
    let runtime = runtime.clone();

    Rc::new(move |library_path| {
        let task = {
            let mut runtime = runtime.borrow_mut();
            runtime.prepare_library_scan(library_path)?
        };

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _sent = tx.send(run_library_scan_task(task));
        });

        poll_library_scan(rx, runtime.clone(), library_changed.clone());
        Ok(())
    })
}

fn poll_library_scan(
    rx: mpsc::Receiver<Result<super::LibraryScanResult, ApplicationRuntimeError>>,
    runtime: SharedRuntime,
    library_changed: LibraryChangedCallback,
) {
    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(Ok(result)) => {
            runtime.borrow_mut().apply_library_scan_result(result);
            library_changed();
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            runtime.borrow_mut().fail_library_scan(error);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => {
            // The runtime's notification observer republishes the
            // "Cancelling..." state on every cancellation-flag flip;
            // we no longer need to poke the widget from here.
            glib::ControlFlow::Continue
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            runtime
                .borrow_mut()
                .fail_library_scan(ApplicationRuntimeError::LibraryScanFailed);
            glib::ControlFlow::Break
        }
    });
}
