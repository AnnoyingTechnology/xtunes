// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{path::PathBuf, rc::Rc, sync::mpsc, time::Duration};

use gtk::glib;

use super::{
    ApplicationRuntimeError, LibraryChangedCallback, SharedRuntime, run_library_scan_task,
    status_bar::StatusBar,
};

pub(crate) type LibraryScanRequestedCallback =
    Rc<dyn Fn(PathBuf) -> Result<(), ApplicationRuntimeError>>;

pub(crate) fn library_scan_requested_callback(
    runtime: &SharedRuntime,
    library_changed: LibraryChangedCallback,
    status_bar: &StatusBar,
) -> LibraryScanRequestedCallback {
    let runtime = runtime.clone();
    let status_bar = status_bar.clone();

    Rc::new(move |library_path| {
        let task = {
            let mut runtime = runtime.borrow_mut();
            let task = runtime.prepare_library_scan(library_path)?;
            status_bar.update_task(runtime.background_task_status());
            task
        };

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _sent = tx.send(run_library_scan_task(task));
        });

        poll_library_scan(
            rx,
            runtime.clone(),
            library_changed.clone(),
            status_bar.clone(),
        );
        Ok(())
    })
}

fn poll_library_scan(
    rx: mpsc::Receiver<Result<super::LibraryScanResult, ApplicationRuntimeError>>,
    runtime: SharedRuntime,
    library_changed: LibraryChangedCallback,
    status_bar: StatusBar,
) {
    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(Ok(result)) => {
            {
                runtime.borrow_mut().apply_library_scan_result(result);
            }
            library_changed();
            status_bar.update_task(runtime.borrow().background_task_status());
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            {
                runtime.borrow_mut().fail_library_scan(error);
            }
            status_bar.update_task(runtime.borrow().background_task_status());
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            {
                runtime
                    .borrow_mut()
                    .fail_library_scan(ApplicationRuntimeError::LibraryScanFailed);
            }
            status_bar.update_task(runtime.borrow().background_task_status());
            glib::ControlFlow::Break
        }
    });
}
