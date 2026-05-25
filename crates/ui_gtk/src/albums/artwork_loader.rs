// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Background loader for album cover artwork.
//!
//! Reading artwork from an audio file is a synchronous, disk- and
//! CPU-bound operation (a `lofty` tag parse plus a pixbuf decode plus a
//! palette derivation). Doing any part of it inline while building
//! album tiles meant the Albums view paid those costs on the GTK main
//! thread, freezing the UI for several seconds on libraries with a few
//! thousand albums.
//!
//! `AlbumArtworkLoader` separates that work:
//!
//! * A small pool of worker threads consumes path requests from a shared
//!   queue and runs the **entire** decode pipeline off the main thread:
//!   `MetadataService::read_artwork` to pull the bytes from the audio
//!   file, `Pixbuf::from_read` to decode them, `ArtworkPalette::from_pixbuf`
//!   to derive the panel palette, and bounded `gdk::Texture`s for the tile
//!   and detail sizes. The pixbuf itself is dropped on the worker; only the
//!   finished `DecodedArtwork` is handed back. (This relies on `gdk::Texture`
//!   being `Send + Sync` in gtk-rs — it is, because GdkTexture is documented
//!   as immutable after construction.)
//! * A GTK main-loop poller drains the result channel under a strict
//!   per-tick budget (small max batch + short wall-clock cap) so even a
//!   burst of completions can't monopolise the main thread, places each
//!   result in the path-keyed cache, and fires the callbacks that were
//!   waiting for that path.
//! * A monotonic *generation* counter lets the view discard callbacks
//!   that belong to a previous rebuild. `begin_generation` is called at
//!   the start of every full tile-grid rebuild; any callback whose
//!   generation no longer matches is dropped without invocation, so
//!   stale results never touch widgets that have been removed.
//! * The cache is keyed by `PathBuf` because the bytes embedded in a
//!   given audio file are stable regardless of which album the view
//!   currently groups it under, which lets the detail panel reuse what
//!   the tile loader already produced.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use gtk::{gdk, gdk_pixbuf, glib};
use sustain_app_runtime::MetadataService;

use crate::artwork_color::ArtworkPalette;

use super::{ALBUM_DETAIL_ARTWORK_SIZE, ALBUM_TILE_COVER_SIZE};

/// Number of worker threads pulling from the request queue. Artwork
/// extraction is dominated by file I/O, tag parsing, and pixbuf decode;
/// a small fixed pool keeps the disk busy and uses a couple of cores
/// for decode without making us a noticeable burden on other processes.
/// Not user-configurable: the right value lives in a narrow band and
/// the wrong one is unlikely to surprise anyone.
const WORKER_COUNT: usize = 4;

/// Poll interval for delivering completed loads back to the main loop.
/// 50ms is fast enough that artwork pops in smoothly while keeping the
/// idle GTK loop quiet when nothing is being decoded.
const RESULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Hard cap on the number of completed results the poller hands to
/// widget callbacks in a single tick. Even when each callback is cheap
/// (texture swap on a single image widget), the cumulative GTK redraw
/// cost can stall the frame if a flood of results lands at once. The
/// cap and the time budget below act together: whichever fires first
/// returns control to the main loop and lets GTK paint a frame before
/// the next batch is delivered.
const RESULT_BATCH_MAX: usize = 8;

/// Wall-clock budget for a single poller tick. Backstops the batch cap
/// against pathological per-callback work — if a single callback ends
/// up doing more than expected, we still relinquish the main thread on
/// schedule rather than running through the whole batch.
const RESULT_TICK_BUDGET: Duration = Duration::from_millis(4);
const TILE_TEXTURE_MAX_SIDE: i32 = ALBUM_TILE_COVER_SIZE;
const DETAIL_TEXTURE_MAX_SIDE: i32 = ALBUM_DETAIL_ARTWORK_SIZE;

/// Decoded artwork shared between tile rendering (needs only the
/// texture) and detail-panel rendering (also needs the palette to tint
/// the panel background/text). Both are computed once per file and
/// cached.
#[derive(Clone, Default)]
pub(super) struct DecodedArtwork {
    pub(super) tile_texture: Option<gdk::Texture>,
    pub(super) detail_texture: Option<gdk::Texture>,
    pub(super) palette: Option<ArtworkPalette>,
}

pub(super) type ArtworkCallback = Box<dyn FnOnce(DecodedArtwork) + 'static>;

#[derive(Clone)]
pub(super) struct AlbumArtworkLoader {
    inner: Rc<LoaderInner>,
}

struct LoaderInner {
    request_tx: mpsc::Sender<WorkerRequest>,
    current_generation: Cell<u64>,
    cache: RefCell<HashMap<PathBuf, DecodedArtwork>>,
    pending: RefCell<HashMap<PathBuf, Vec<PendingCallback>>>,
}

struct WorkerRequest {
    path: PathBuf,
}

struct WorkerResult {
    path: PathBuf,
    decoded: DecodedArtwork,
}

struct PendingCallback {
    generation: u64,
    callback: ArtworkCallback,
}

impl AlbumArtworkLoader {
    pub(super) fn new(metadata_service: Arc<dyn MetadataService>) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<WorkerRequest>();
        let request_rx = Arc::new(Mutex::new(request_rx));
        let (result_tx, result_rx) = mpsc::channel::<WorkerResult>();

        for index in 0..WORKER_COUNT {
            let request_rx = Arc::clone(&request_rx);
            let result_tx = result_tx.clone();
            let metadata_service = Arc::clone(&metadata_service);
            thread::Builder::new()
                .name(format!("sustain-artwork-{index}"))
                .spawn(move || worker_loop(request_rx, result_tx, metadata_service))
                .expect("spawn artwork worker thread");
        }
        // Workers each keep their own clone of the result sender; drop
        // the original so the poller's `Disconnected` actually fires
        // when every worker has exited.
        drop(result_tx);

        let inner = Rc::new(LoaderInner {
            request_tx,
            current_generation: Cell::new(0),
            cache: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashMap::new()),
        });

        install_result_poller(Rc::clone(&inner), result_rx);

        Self { inner }
    }

    /// Begin a new generation. All previously queued callbacks are
    /// dropped without firing — their target widgets belong to a rebuild
    /// that has been superseded. In-flight worker reads may still
    /// complete; their results land in the cache (useful for the next
    /// rebuild) but trigger no callbacks. Returns the new generation;
    /// the view passes it back into each `request` call so the loader
    /// can tell whether a callback is still relevant when its result
    /// arrives.
    pub(super) fn begin_generation(&self) -> u64 {
        let next = self.inner.current_generation.get().wrapping_add(1);
        self.inner.current_generation.set(next);
        self.inner.pending.borrow_mut().clear();
        next
    }

    /// Current generation used by newly-bound virtual rows. Rebuilds advance
    /// the generation once, then every visible row binding queues artwork
    /// requests against that stable value until the next rebuild.
    pub(super) fn current_generation(&self) -> u64 {
        self.inner.current_generation.get()
    }

    /// Returns the decoded entry for `path`, if any. Lets the detail
    /// panel reuse what the tile loader already produced rather than
    /// reading the file a second time.
    pub(super) fn cached(&self, path: &Path) -> Option<DecodedArtwork> {
        self.inner.cache.borrow().get(path).cloned()
    }

    /// Request the decoded artwork for `path`. The callback fires on
    /// the main thread when the artwork becomes available, unless the
    /// loader has advanced past `generation` in the meantime — in that
    /// case the callback is dropped silently. Cache hits fire
    /// synchronously, so a tile whose neighbour just resolved the same
    /// file never schedules redundant disk work.
    pub(super) fn request(&self, generation: u64, path: PathBuf, callback: ArtworkCallback) {
        if generation < self.inner.current_generation.get() {
            return;
        }
        if let Some(cached) = self.inner.cache.borrow().get(&path) {
            callback(cached.clone());
            return;
        }
        let mut pending = self.inner.pending.borrow_mut();
        let needs_queue = !pending.contains_key(&path);
        pending
            .entry(path.clone())
            .or_default()
            .push(PendingCallback {
                generation,
                callback,
            });
        if needs_queue {
            // Send only fails if every worker has exited, which happens
            // exclusively at shutdown. Drop the callback silently in
            // that case — there is no view left to update.
            let _ = self.inner.request_tx.send(WorkerRequest { path });
        }
    }

    /// Synchronously read and cache the artwork at `path`. Used by the
    /// album detail panel when the user clicks an album whose tile
    /// hasn't been resolved yet — the panel needs the palette to render
    /// at all, and a single tag read is fast enough that blocking the
    /// click for one file is preferable to flashing colours in after
    /// the fact. Subsequent loader callbacks for the same path see the
    /// cache hit.
    pub(super) fn ensure_cached_sync(
        &self,
        path: &Path,
        metadata_service: &dyn MetadataService,
    ) -> DecodedArtwork {
        if let Some(cached) = self.inner.cache.borrow().get(path) {
            return cached.clone();
        }
        let bytes = metadata_service.read_artwork(path).ok().flatten();
        let decoded = decode_artwork(bytes);
        self.inner
            .cache
            .borrow_mut()
            .insert(path.to_path_buf(), decoded.clone());
        decoded
    }
}

fn worker_loop(
    request_rx: Arc<Mutex<mpsc::Receiver<WorkerRequest>>>,
    result_tx: mpsc::Sender<WorkerResult>,
    metadata_service: Arc<dyn MetadataService>,
) {
    loop {
        let request = {
            // Hold the lock only long enough to take one item; the
            // expensive read and decode happen unlocked so the other
            // workers can pull from the queue concurrently.
            let Ok(rx) = request_rx.lock() else {
                return;
            };
            match rx.recv() {
                Ok(request) => request,
                Err(_) => return,
            }
        };
        let bytes = metadata_service.read_artwork(&request.path).ok().flatten();
        let decoded = decode_artwork(bytes);
        if result_tx
            .send(WorkerResult {
                path: request.path,
                decoded,
            })
            .is_err()
        {
            return;
        }
    }
}

fn install_result_poller(inner: Rc<LoaderInner>, rx: mpsc::Receiver<WorkerResult>) {
    glib::timeout_add_local(RESULT_POLL_INTERVAL, move || {
        // Bound per-tick work so a burst of completed loads doesn't
        // monopolise the main thread. Stop on whichever fires first:
        // the batch cap, the wall-clock budget, an empty queue, or all
        // workers having exited.
        let started = std::time::Instant::now();
        let mut processed = 0;
        loop {
            if processed >= RESULT_BATCH_MAX || started.elapsed() >= RESULT_TICK_BUDGET {
                return glib::ControlFlow::Continue;
            }
            match rx.try_recv() {
                Ok(result) => {
                    inner
                        .cache
                        .borrow_mut()
                        .insert(result.path.clone(), result.decoded.clone());
                    let callbacks = inner
                        .pending
                        .borrow_mut()
                        .remove(&result.path)
                        .unwrap_or_default();
                    let current = inner.current_generation.get();
                    for entry in callbacks {
                        if entry.generation == current {
                            (entry.callback)(result.decoded.clone());
                        }
                    }
                    processed += 1;
                }
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
            }
        }
    });
}

fn decode_artwork(bytes: Option<Vec<u8>>) -> DecodedArtwork {
    let Some(bytes) = bytes else {
        return DecodedArtwork::default();
    };
    let pixbuf = match gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes)) {
        Ok(pixbuf) => pixbuf,
        Err(_) => return DecodedArtwork::default(),
    };
    DecodedArtwork {
        tile_texture: scaled_texture(&pixbuf, TILE_TEXTURE_MAX_SIDE),
        detail_texture: scaled_texture(&pixbuf, DETAIL_TEXTURE_MAX_SIDE),
        palette: ArtworkPalette::from_pixbuf(&pixbuf),
    }
}

fn scaled_texture(pixbuf: &gdk_pixbuf::Pixbuf, max_side: i32) -> Option<gdk::Texture> {
    let width = pixbuf.width();
    let height = pixbuf.height();
    if width <= 0 || height <= 0 || max_side <= 0 {
        return None;
    }

    let largest_side = width.max(height);
    let scale = (f64::from(max_side) / f64::from(largest_side)).min(1.0);
    let target_width = (f64::from(width) * scale).round().max(1.0) as i32;
    let target_height = (f64::from(height) * scale).round().max(1.0) as i32;

    let scaled = if target_width == width && target_height == height {
        pixbuf.clone()
    } else {
        pixbuf.scale_simple(
            target_width,
            target_height,
            gdk_pixbuf::InterpType::Bilinear,
        )?
    };
    Some(gdk::Texture::for_pixbuf(&scaled))
}
