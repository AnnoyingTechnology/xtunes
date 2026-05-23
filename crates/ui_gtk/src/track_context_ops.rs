// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{FileLauncher, gdk, gio};
use xtunes_app_runtime::TrackId;

use super::{SharedRuntime, track_context::TrackActionCallback};

pub(crate) fn copy_files_callback(
    runtime: &SharedRuntime,
    window: &gtk::Window,
) -> TrackActionCallback {
    let runtime = runtime.clone();
    let window = window.clone();
    Rc::new(move |track_ids: Vec<TrackId>| {
        let paths = absolute_paths_for_tracks(&runtime, &track_ids);
        if paths.is_empty() {
            return;
        }
        copy_paths_to_clipboard(&window, &paths);
    })
}

pub(crate) fn show_in_folder_callback(
    runtime: &SharedRuntime,
    window: &gtk::Window,
) -> TrackActionCallback {
    let runtime = runtime.clone();
    let window = window.clone();
    Rc::new(move |track_ids: Vec<TrackId>| {
        let Some(path) = absolute_paths_for_tracks(&runtime, &track_ids)
            .into_iter()
            .next()
        else {
            return;
        };
        show_path_in_folder(&window, &path);
    })
}

fn absolute_paths_for_tracks(runtime: &SharedRuntime, track_ids: &[TrackId]) -> Vec<PathBuf> {
    let runtime_borrow = runtime.borrow();
    let tracks = runtime_borrow.library_tracks();
    track_ids
        .iter()
        .filter_map(|track_id| {
            tracks
                .iter()
                .find(|track| track.id == *track_id)
                .and_then(|track| runtime_borrow.absolute_track_path(track))
        })
        .collect()
}

fn copy_paths_to_clipboard(window: &gtk::Window, paths: &[PathBuf]) {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    let file_list = gdk::FileList::from_array(&files);
    let provider = gdk::ContentProvider::for_value(&file_list.to_value());
    if let Err(error) = window.clipboard().set_content(Some(&provider)) {
        eprintln!("xtunes: clipboard set failed: {error}");
    }
}

fn show_path_in_folder(window: &gtk::Window, path: &Path) {
    // @TODO codex: confirm gtk::FileLauncher::open_containing_folder is the
    // right primitive here. Internally it calls org.freedesktop.FileManager1
    // ShowItems over D-Bus and falls back to opening the parent directory if
    // that fails — which is the behaviour we want — but worth a second pair
    // of eyes on the choice vs. calling D-Bus directly.
    let file = gio::File::for_path(path);
    let launcher = FileLauncher::new(Some(&file));
    launcher.open_containing_folder(Some(window), None::<&gio::Cancellable>, |result| {
        if let Err(error) = result {
            eprintln!("xtunes: open containing folder failed: {error}");
        }
    });
}
