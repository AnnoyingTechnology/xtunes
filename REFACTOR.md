# Refactor audit — Sustain workspace

Snapshot taken 2026-05-27. Rhythmbox exporter excluded. Numbers are `wc -l`;
"prod" = everything above the first `#[cfg(test)]`, "tests" = the rest.

## Size breakdown (prod vs test)

| File | Total | Prod | Tests | Test % |
| --- | ---: | ---: | ---: | ---: |
| `crates/app_runtime/src/lib.rs` | 4 129 | 939 | 3 190 | 77% |
| `crates/ui_gtk/src/main_window.rs` | 2 684 | 2 592 | 92 | 3% |
| `crates/library_store/src/lib.rs` | 2 978 | 1 330 | 1 648 | 55% |
| `crates/app_runtime/src/managed_library.rs` | 1 577 | 1 453 | 124 | 8% |
| `crates/ui_gtk/src/track_table/cells.rs` | 1 196 | 1 162 | 34 | 3% |
| `crates/ui_gtk/src/albums.rs` | 1 181 | 1 158 | 23 | 2% |
| `crates/ui_gtk/src/now_playing.rs` | 996 | 996 | 0 | 0% |
| `crates/ui_gtk/src/track_context.rs` | 875 | 696 | 179 | 20% |
| `crates/ui_gtk/src/smart_playlist_editor/model.rs` | 865 | 645 | 220 | 25% |
| `crates/metadata/src/lib.rs` | 832 | 571 | 261 | 31% |
| `crates/app_runtime/src/analysis_scheduler.rs` | 816 | 381 | 435 | 53% |
| `crates/ui_gtk/src/artwork_loader.rs` | 799 | 799 | 0 | 0% |
| `crates/app_runtime/src/notifications.rs` | 668 | 444 | 224 | 34% |
| `crates/library_store/src/sqlite_rows.rs` | 658 | 658 | 0 | 0% |
| `crates/domain/src/playback/queue.rs` | 600 | 310 | 290 | 48% |
| `crates/domain/src/smart_playlist_evaluation.rs` | 573 | 271 | 302 | 53% |

Crate file counts: `ui_gtk` 50, `domain` 27, `app_runtime` 14,
`metadata_remote` 9, `library_store` 5, `analysis` 5, `desktop` 3, `app` 2,
`{settings, search, playback, metadata}` 1 each.

---

## Tier 1 — extract tests to sibling files

Idiomatic Rust pattern, zero behavioural risk:

```rust
#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
```

The repo has no existing convention for split test files — adopting one
establishes a uniform pattern as a side benefit. Integration tests under a
`tests/` directory are not a substitute: most of these tests reach into
private items via `super::*` and must stay in-crate.

Apply to:

| File | Move to | Prod after | Tests file |
| --- | --- | ---: | ---: |
| `crates/app_runtime/src/lib.rs` | `src/lib_tests.rs` | 939 | 3 190 |
| `crates/library_store/src/lib.rs` | `src/lib_tests.rs` | 1 330 | 1 648 |
| `crates/app_runtime/src/analysis_scheduler.rs` | `src/analysis_scheduler_tests.rs` | 381 | 435 |
| `crates/app_runtime/src/notifications.rs` | `src/notifications_tests.rs` | 444 | 224 |
| `crates/domain/src/playback/queue.rs` | `src/playback/queue_tests.rs` | 310 | 290 |
| `crates/domain/src/smart_playlist_evaluation.rs` | `src/smart_playlist_evaluation_tests.rs` | 271 | 302 |
| `crates/ui_gtk/src/smart_playlist_editor/model.rs` | sibling | 645 | 220 |
| `crates/metadata/src/lib.rs` | sibling | 571 | 261 |

After this pass the workspace's largest file drops from 4 129 to 2 592 lines
(`main_window.rs`), and `app_runtime/lib.rs` falls out of the "concerning"
bucket entirely.

---

## Tier 2 — production-code refactors

### `crates/ui_gtk/src/main_window.rs` — 2 592 prod lines

After Tier 1 this is the workspace's largest prod file. Shape: one 487-line
constructor (`build_main_window`, L75–L562) plus ~50 free `*_callback` /
`install_*` helpers below it. The constructor is sequential composition —
leave it alone. The helpers cluster cleanly by topic and should move into
sibling modules:

| Topic | Helpers | Suggested module |
| --- | --- | --- |
| Search wiring | `install_search_wiring`, `SearchWiringContext` | `main_window/search.rs` |
| MPRIS bridge | `install_mpris_command_consumer`, `handle_mpris_command`, `now_playing_to_mpris_metadata` | `main_window/mpris_bridge.rs` |
| Sidebar callbacks | `sidebar_*_callback`, `resolve_move_target` (~7 fns) | `main_window/sidebar_callbacks.rs` |
| Track row / table | `track_row_changed_callback`, `rating_changed_callback`, `*_track_activated_callback`, reorder helpers | `main_window/track_callbacks.rs` |
| Playlist wiring | `install_playlists_view_activator`, `playlist_table_rows_for`, `add_to_playlist_*`, `make_playlists_header_play_callback` | `main_window/playlists.rs` |
| Result consumers | `install_metadata_write_result_consumer`, `install_analysis_progress_consumer`, `install_artwork_fetch_result_consumer` | `main_window/result_consumers.rs` |
| Keyboard | `install_keyboard_shortcuts`, `KeyboardShortcutContext`, `jump_to_current_track` | fold into existing `shortcuts.rs` sibling |

Target: `main_window.rs` ≤ ~700 lines.

### `crates/app_runtime/src/managed_library.rs` — 1 453 prod lines

Three concerns at three abstraction levels share one file. Split:

- `managed_library/import.rs` — `LibraryImportContext` (~310 lines), planning
  helpers (`plan_destination`, `reference_relative_path_for_source`,
  `source_relative_path_inside_library`, `collect_supported_audio_*`).
- `managed_library/consolidation.rs` — `LibraryConsolidationContext` (~90 lines),
  `LibraryConsolidationPlan`, `PlannedLibraryConsolidationMove`,
  `plan_library_consolidation`, `plan_managed_track_retarget`,
  `plan_consolidation_destination`.
- `managed_library/journal.rs` — `ConsolidationJournalEntry`,
  `recover_consolidation_journal_entry`, `save_recovered_consolidation_track`,
  `write_consolidation_journal`, `read_consolidation_journal`,
  `remove_consolidation_journal_if_present`, `consolidation_journal_path`,
  `temporary_consolidation_journal_path`, relative-path codec
  (`encode_relative_path`, `decode_relative_path`, `hex_value`).
- `managed_library/file_ops.rs` — `VerifiedFileCopy`, `VerifiedFileCopyError`,
  `copy_file_verified`, `copy_file_verified_inner`, `copy_to_temporary_file`,
  `create_temporary_copy_path`, `move_file_without_copy_or_overwrite`,
  `rollback_file_move`, `path_is_regular_file`, `paths_refer_to_same_file`.
- `managed_library.rs` — keeps the public entry points and inherent methods.

Target: each module 250–400 lines.

### `crates/library_store/src/lib.rs` — 1 330 prod lines (after test extraction)

The `impl LibraryStore for SqliteLibraryStore` block is 741 lines and groups
cleanly by table. Split into sibling modules with multiple `impl` blocks (no
ceremony required in Rust):

- `sqlite/tracks.rs` — `save_track`, `save_tracks`, `delete_track`, `track`,
  `track_by_content_hash`, `tracks`, plus `save_track_with_connection` and
  `upsert_track_analysis`.
- `sqlite/playlists.rs` — `save_playlist`, `playlist`, `playlists`,
  `delete_playlist`, `save_playlist_folder`, `playlist_folder`,
  `playlist_folders`, `delete_playlist_folder`.
- `sqlite/smart_playlists.rs` — `save_smart_playlist` (75 lines),
  `smart_playlist`, `smart_playlists`, `delete_smart_playlist`.
- `sqlite/column_layouts.rs` — `load_track_column_layout`,
  `save_track_column_layout` (84 lines), `delete_track_column_layout`,
  `load_layout_rows`.
- `sqlite/analysis.rs` — `record_analysis`, `record_analysis_attempt_failure`,
  `tracks_needing_analysis`, `load_waveform`.
- `sqlite/synced_lyrics.rs` — `record_synced_lyrics`, `load_synced_lyrics`,
  `clear_synced_lyrics`.
- `lib.rs` — keeps the trait, the `SqliteLibraryStore` struct + `new`,
  public types (`AnalysisCapabilities`, `StoredWaveform`, `AnalysisContext`,
  `StoredSyncedLyrics`), `StoreError`, `default_database_path`.

Note: `InMemoryLibraryStore` is an intentional twin impl for tests — keep it.

Target: `lib.rs` ≤ 400 lines.

---

## Tier 3 — opportunistic, as files get touched

### `crates/ui_gtk/src/albums.rs` — 1 158 prod lines

`AlbumsView` impl is 716 lines. Two clean extractions:

- `albums/detail.rs` — `album_detail_arrow_row`, `album_detail_arrow`,
  `album_detail_palette_provider`, `apply_palette_style`,
  `install_palette_provider`, `album_detail_palette_css`,
  `detail_icon_button`.
- `albums/cover.rs` — `build_cover_widget`, `apply_cover_texture`,
  `album_cover_with`, `album_cover_placeholder`, `empty_tile_placeholder`.

### `crates/ui_gtk/src/track_table/cells.rs` — 1 162 prod lines

Cell factories and drag/drop machinery share one file. Extract:

- `track_table/drag_drop.rs` — `install_cell_drop_target`,
  `drop_position_from_offset`, `drop_would_self_target`,
  `install_cell_drag_source`, `build_drag_paintable`, `find_listview_row`,
  `visible_selected_row_widgets`, `compose_stacked_row_paintable`,
  `RowDropCellEntry`, `RowDropCellRegistry`.

### `crates/ui_gtk/src/now_playing.rs` — 996 prod lines

Self-contained ~280-line marquee subsystem embedded in a UI file
(`MarqueeLabel`, `MarqueeDrawModel`, `MarqueeTextStyle`,
`install_marquee_draw_func`, `draw_marquee_text`, `set_context_color`,
`set_text_source`, `draw_text_at`). Extract to `now_playing/marquee.rs`.

### `crates/ui_gtk/src/artwork_loader.rs` — 799 prod lines

Three concerns in one file:

- Loader / worker thread (`ArtworkLoader`, `LoaderInner`, `WorkerRequest`,
  `WorkerResult`, `worker_loop`, `install_result_poller`).
- Repository / disk cache (`ArtworkRepository` ~33 lines,
  `ArtworkDiskCache` ~210 lines, `CachedArtwork`, `CachedArtworkRow`).
- Decode/scale helpers (`decode_artwork`, `scaled_pixbuf`, `pixbuf_png_bytes`,
  `texture_from_png`, `palette_components_from_cache_row`,
  `rgb_from_cache_columns`).

Split into `artwork_loader/{loader.rs, disk_cache.rs, decode.rs}`.

---

# Duplicated implementations

Findings verified by direct comparison of function bodies.

## Two parallel scheduler shells — OBSOLETE (schedulers diverged)

> **Resolution (2026-05-29): no harness.** The premise below no longer holds.
> Since this snapshot the analysis scheduler was rewritten into a
> multi-threaded `WorkerPool` + `supervisor_loop` with resource-usage presets:
> it gained a `ResourceUsageChanged` command, a richer `SupervisorState` that
> threads a `pool` through `drain_commands`/`apply_command`, efficiency-core
> pinning, and an MPMC dispatch queue. The online scheduler is still a single
> sequential `worker_loop`. `drain_commands`/`apply_command` now have different
> signatures, command sets, and state structs — they are no longer identical.
>
> A generic `SchedulerHarness` over the *execution loop* would have to either
> force the online path into a pool model (over-engineering rate-limited
> network I/O), force the analysis path back into a single loop (a performance
> regression, forbidden by the perf rules), or abstract over "single-loop vs
> pool" (a leaky over-abstraction). None is acceptable, so the harness is
> dropped.
>
> What remains genuinely duplicated is narrow: the `SchedulerProgress` enum
> (identical, and the UI deliberately relies on the shapes matching) and the
> `ProgressSink`/`TrackUpdatedSink`/`UnixClockFn` aliases, plus the
> `{ sender, handle }` struct with its byte-identical `shutdown`/`Drop`
> discipline. These are leaf-level and not worth a generic abstraction given
> the divergence; left as-is by maintainer decision.
>
> `smart_shuffle_scheduler.rs` (added after this snapshot) is a different
> design entirely — a request-driven async rebuild over `async_channel` — and
> was never part of this duplication.

The original (now-stale) analysis is preserved below for history.

`crates/app_runtime/src/analysis_scheduler.rs` and
`crates/app_runtime/src/online_scheduler.rs` share large parts of their
scaffolding byte-for-byte. The second was clearly built by copying the first.

Verified parallels:

| Item | `analysis_scheduler.rs` | `online_scheduler.rs` | Verdict |
| --- | --- | --- | --- |
| `pub enum SchedulerProgress { Tick { completed, failed, remaining }, Idle { completed, failed } }` | L77 | L68 | Same shape, identical fields |
| `pub struct *SchedulerConfig` | L96 | L81 | Same role, different deps |
| `enum SchedulerCommand { SettingsChanged(_), LibraryPathChanged(_), Wake, Shutdown }` | L108 | L93 | Same 4 variants, only generic param differs |
| `pub struct *Scheduler { sender, handle }` | L117 | L102 | Same 2 fields |
| `start()` constructor (mpsc + named `thread::Builder` + `worker_loop`) | L123 | L108 | Same pattern |
| `impl Drop` (best-effort `Shutdown` send + sender replace) | L167 | L147 | Same body, only comment differs |
| `struct WorkerState { settings, library_path, completed, failed }` | L334 | L487 | Same 4 fields |
| `enum DrainOutcome { Continue, Shutdown }` | L341 | L494 | Identical |
| `fn drain_commands` | L346 | L499 | **Byte-for-byte identical** |
| `fn apply_command` | L359 | L512 | **Byte-for-byte identical** |

What actually differs: only the per-iteration work (`process_track`), the
settings type (`AnalysisSettings` vs `OnlineSettings`), the capabilities
type, and the thread-name string.

## Three `url_encode` functions in `metadata_remote/`

| File | Function | Body |
| --- | --- | --- |
| `crates/metadata_remote/src/musicbrainz.rs:182` | `url_encode` | Iterates bytes, keeps RFC 3986 unreserved set literal, percent-encodes the rest as `%XX` |
| `crates/metadata_remote/src/lrclib.rs:122` | `url_encode` | Same body, only the doc comment differs |
| `crates/metadata_remote/src/acoustid.rs:128` | `url_encode_query_component` | Same body, renamed |

Fix: single shared helper in a `metadata_remote/src/http.rs` module (or
`pub(crate)` in `lib.rs`), called three times. A future bug fix to encoding
behaviour currently has to be made in three places, and silent divergence
between providers is a real risk.

## Three identical `non_empty_text` helpers

Byte-for-byte identical bodies in:

- `crates/ui_gtk/src/albums/model.rs:266`
- `crates/ui_gtk/src/now_playing/model.rs:81`
- `crates/ui_gtk/src/track_table/row.rs:116`

```rust
fn non_empty_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
```

Fix: move to a shared `crates/ui_gtk/src/util.rs`.

## Three near-identical `normalized_*_name` helpers

| File | Line | Function | Error variant |
| --- | --- | --- | --- |
| `crates/app_runtime/src/playlists.rs` | 230 | `normalized_playlist_name` | `InvalidPlaylistName` |
| `crates/app_runtime/src/playlist_folders.rs` | 76 | `normalized_folder_name` | `InvalidPlaylistFolderName` |
| `crates/app_runtime/src/smart_playlists.rs` | 100 | `normalized_smart_playlist_name` | `InvalidSmartPlaylistName` |

All three bodies: `name.trim().to_owned()`, error if empty, otherwise `Ok`.
Only the error variant differs.

Fix: single helper taking the error constructor as a fn pointer:

```rust
pub(crate) fn normalized_name(
    name: String,
    on_empty: fn() -> ApplicationRuntimeError,
) -> ApplicationRuntimeResult<String> { … }
```

## Two `compare_optional_text` functions

| File | Line | Body |
| --- | --- | --- |
| `crates/library_store/src/query.rs` | 68 | Inline: trim + `to_ascii_lowercase` on both sides, then `cmp` |
| `crates/search/src/lib.rs` | 102 | Delegates to a local `fn normalize` (same trim + `to_ascii_lowercase`) |

Both compute identical orderings on identical inputs.

Fix: pick one home (`search` is the natural fit; `domain` works too since
the comparison is pure-data) and have the other crate call it. Risk if left
alone: a future change to one normaliser (say, unicode case folding) will
silently produce different sort orders for the same query depending on
codepath.

## Two `sync_rating_button` functions

| File | Line | Body |
| --- | --- | --- |
| `crates/ui_gtk/src/track_info/details.rs:272` | `fn sync_rating_button(button, star, rating)` | Sets label to filled/empty star unicode literal |
| `crates/ui_gtk/src/track_table/cells.rs:1135` | `fn sync_rating_button(button, star, rating)` | Same label logic via `rating_star_label`, **plus** toggles `rating-star-filled` / `rating-star-empty` CSS classes |

The `details.rs` version is a degenerate copy of the `cells.rs` one: same
label logic, missing the CSS-class toggling. The track-info detail page
therefore can't be styled the way the table cells are. Likely an oversight,
not an intentional difference.

Fix: extract one renderer (in a shared module), use it from both call sites.
The detail page picks up consistent styling for free.

---

## Suggested order of work

1. **Tier 1 (test extraction)** — eight files, purely mechanical, no
   behavioural risk. Drops the workspace's two largest files by ~3 200 and
   ~1 600 lines.
2. **Small duplication fixes** — `non_empty_text`, `normalized_*_name`,
   `compare_optional_text`, `sync_rating_button`. Quick wins, each unblocks
   a future divergence risk.
3. **`url_encode` consolidation** in `metadata_remote/`.
4. **`library_store/src/lib.rs`** trait-impl split by table.
5. **`managed_library.rs`** — extract `journal.rs` and `file_ops.rs` first
   (cleanly self-contained), then split import vs. consolidation.
6. ~~**Scheduler harness**~~ — **obsolete.** The analysis and online
   schedulers diverged architecturally after this snapshot (see "Two parallel
   scheduler shells — OBSOLETE" above); a unified execution-loop harness is no
   longer viable without a regression or a leaky abstraction. Skipped by
   maintainer decision.
7. **`main_window.rs`** topic-grouped callback extraction.
8. **Tier 3** — opportunistic, as those files get touched for feature work.
