// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{path::PathBuf, rc::Rc, sync::mpsc, time::Duration};

use gtk::glib;
use gtk::prelude::*;
use gtk::{gdk, gio};

use super::{
    ApplicationRuntimeError, LibraryChangedCallback, SharedRuntime, run_library_import_task,
    status_bar::StatusBar,
};

pub(crate) type LibraryImportRequestedCallback =
    Rc<dyn Fn(Vec<PathBuf>) -> Result<(), ApplicationRuntimeError>>;

pub(crate) fn library_import_requested_callback(
    runtime: &SharedRuntime,
    library_changed: LibraryChangedCallback,
    status_bar: &StatusBar,
) -> LibraryImportRequestedCallback {
    let runtime = runtime.clone();
    let status_bar = status_bar.clone();

    Rc::new(move |paths| {
        let task = {
            let mut runtime = runtime.borrow_mut();
            let task = runtime.prepare_library_import(paths)?;
            status_bar.update_task(runtime.background_task_status());
            task
        };

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _sent = tx.send(run_library_import_task(task));
        });

        poll_library_import(
            rx,
            runtime.clone(),
            library_changed.clone(),
            status_bar.clone(),
        );
        Ok(())
    })
}

pub(crate) fn install_file_drop_target(
    drop_zone: &impl IsA<gtk::Widget>,
    drop_indicator: &impl IsA<gtk::Widget>,
    import_requested: LibraryImportRequestedCallback,
) {
    let drop_target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);

    let indicator_for_enter = drop_indicator.clone().upcast::<gtk::Widget>();
    drop_target.connect_enter(move |_target, _x, _y| {
        indicator_for_enter.add_css_class(LIBRARY_DROP_ACTIVE_CLASS);
        gdk::DragAction::COPY
    });
    let indicator_for_leave = drop_indicator.clone().upcast::<gtk::Widget>();
    drop_target.connect_leave(move |_target| {
        indicator_for_leave.remove_css_class(LIBRARY_DROP_ACTIVE_CLASS);
    });

    drop_target.connect_drop(move |_target, value, _x, _y| {
        let Ok(file_list) = value.get::<gdk::FileList>() else {
            return false;
        };
        let paths = local_paths_from_file_list(&file_list);
        if paths.is_empty() {
            return false;
        }
        import_requested(paths).is_ok()
    });
    drop_zone.add_controller(drop_target);
}

pub(crate) const LIBRARY_DROP_INDICATOR_CLASS: &str = "library-drop-indicator";
const LIBRARY_DROP_ACTIVE_CLASS: &str = "library-drop-active";

fn poll_library_import(
    rx: mpsc::Receiver<Result<super::LibraryImportResult, ApplicationRuntimeError>>,
    runtime: SharedRuntime,
    library_changed: LibraryChangedCallback,
    status_bar: StatusBar,
) {
    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(Ok(result)) => {
            {
                runtime.borrow_mut().apply_library_import_result(result);
            }
            library_changed();
            status_bar.update_task(runtime.borrow().background_task_status());
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            {
                runtime.borrow_mut().fail_library_import(error);
            }
            status_bar.update_task(runtime.borrow().background_task_status());
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            {
                runtime
                    .borrow_mut()
                    .fail_library_import(ApplicationRuntimeError::LibraryImportFailed);
            }
            status_bar.update_task(runtime.borrow().background_task_status());
            glib::ControlFlow::Break
        }
    });
}

fn local_paths_from_file_list(file_list: &gdk::FileList) -> Vec<PathBuf> {
    file_list
        .files()
        .into_iter()
        .filter_map(|file: gio::File| file.path())
        .collect()
}
