# Pending Refactor — Maintainability Audit

Snapshot date: 2026-05-25. Audit covers the `crates/` tree only.

## Line counts (top 12 source files)

| Lines | File | Notes |
|---|---|---|
| 3562 | `crates/app_runtime/src/lib.rs` | **654 prod + 2907 inline tests** — tests dominate the file |
| 1984 | `crates/library_store/src/lib.rs` | 1160 prod + 823 tests; contains 139-line raw `CREATE TABLE` block and 119-line `INSERT/UPDATE` column spam |
| 1966 | `crates/ui_gtk/src/main_window.rs` | ~50 top-level wiring/callback fns glued together |
| 1470 | `crates/app_runtime/src/managed_library.rs` | planner + journal + move runtime + verified-copy mixed |
| 1174 | `crates/ui_gtk/src/albums.rs` | tile grid + detail panel + palette + arrow drawing |
| 955  | `crates/ui_gtk/src/track_table/cells.rs` | context menu + DnD source + DnD target + status icon + rating render |
| 951  | `crates/ui_gtk/src/now_playing.rs` | marquee widget + cairo draw + dominant-color CSS |
| 839  | `crates/ui_gtk/src/track_context.rs` | action registry + popover + trash confirmation |
| 833  | `crates/domain/src/playback.rs` | `PlaybackQueue` + SplitMix64 RNG + `VolumePercent` |
| 831  | `crates/metadata/src/lib.rs` | service + atomic write + format helpers |
| 801  | `crates/ui_gtk/src/smart_playlist_editor/model.rs` | value parsers + operator maps + ISO date math |
| 773  | `crates/ui_gtk/src/artwork_loader.rs` | fingerprint + cache + decode |

## Worst offenders

| # | Symptom | Where | Refactor |
|---|---|---|---|
| 1 | Tests crowding out prod code (4.5× ratio) | `app_runtime/src/lib.rs` (655–3562) | Move `mod tests` to `tests/` integration suite, or distribute into the existing submodules (`commands`, `library_mutation`, `playback`, `playlists`, `smart_playlists`) where each test's subject already lives |
| 2 | 190-line table-driven test as one fn | `app_runtime/src/lib.rs:701` `runtime_handles_every_application_command_intentionally` | Keep the table but extract the `(command, expected)` cases into a `const`/helper; split per-command-family if grouping reveals families |
| 3 | God module of GTK wiring | `ui_gtk/src/main_window.rs` | Already calls into submodules; pull free callbacks into `wiring/{sidebar,search,mpris,keyboard,artwork,track_context}.rs`. Build keeps `main_window.rs` as composition root only |
| 4 | 139-line schema string + 119-line save | `library_store/src/lib.rs:185, 342` | Move schema to `schema.rs` (`pub const SCHEMA_SQL: &str`); for `save_track_with_connection`, define column-name constants and a single source-of-truth column list (current INSERT and UPDATE lists must stay in sync by hand) |
| 5 | Mixed concerns in one file | `app_runtime/src/managed_library.rs` | Split: `import.rs` (managed/referenced add), `consolidate.rs` (planner + move loop), `journal.rs` (recovery), `verified_copy.rs` (already free fn, move with helpers) |
| 6 | DnD + render + menu in one file | `ui_gtk/src/track_table/cells.rs` | Split: `cells/rating.rs`, `cells/status.rs`, `cells/dnd_source.rs`, `cells/dnd_target.rs`, `cells/context_menu_attach.rs` |
| 7 | Marquee widget buried in screen | `ui_gtk/src/now_playing.rs` (101–945) | Marquee label + cairo draw is a self-contained widget; extract to `now_playing/marquee.rs` |
| 8 | Tile grid + detail + palette mixed | `ui_gtk/src/albums.rs` | Split detail panel + palette/arrow drawing out of the grid view |
| 9 | Queue + RNG + volume in domain | `crates/domain/src/playback.rs` | Pre-emptive split before Up Next/Smart Shuffle (PLAN.md §Up Next Queue, §Smart Shuffle) land more state in `PlaybackQueue`: `playback/queue.rs`, `playback/shuffle.rs`, `playback/volume.rs` |
| 10 | `#[allow(clippy::too_many_arguments)]` | `ui_gtk/src/sidebar/row_context.rs:10, 66` | Two suppressions on the same module — group args into the existing `RowContext` struct rather than suppressing |

## Healthy signals

- 1 TODO across the whole tree; no `todo!`/`unimplemented!`.
- No `unwrap()` in prod paths (rough sweep — 500 hits but virtually all in tests).
- No upward crate dependencies (no domain/runtime crate imports `ui_gtk`).
- Sub-tree splits already exist (`ui_gtk/src/{sidebar,track_table,albums,now_playing,track_info,smart_playlist_editor}/`) — proven pattern to extend to (3), (5), (6), (9).

## Priority against PLAN.md work

- Before Up Next / Smart Shuffle: do (9) — `domain/playback.rs` queue split.
- Before Library Scan perf pass: keep `managed_library.rs` as-is until then; (5) pairs naturally with that work.
- Before any further `app_runtime` test additions: do (1) — every new test today widens the lib.rs hot spot.
