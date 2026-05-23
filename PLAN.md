# Sustain Architecture Plan

## Product Name (Open)

The working name `Sustain` is provisional and **must** change before any public
release. The rename is driven by two independent concerns, either of which
would be sufficient on its own:

1. **Taste.** The maintainer is not satisfied with the name: the `x` prefix
   carries a late-90s/early-2000s Linux-desktop flavor (xterm, xmms, xchat,
   xine) that reads as dated rather than as heritage, and the `Tunes` half
   leans too hard on iTunes phonetics for an application that is explicitly
   not iTunes.
2. **Legal exposure.** `Sustain` shares the distinctive `Tunes` suffix with
   Apple's `iTunes` registered trademark (EUIPO + INPI) and the product is
   openly positioned as an iTunes-inspired player. Under EU trademark law
   (Regulation 2017/1001) and the French Code de la propriété intellectuelle
   (art. L713-2/3), the likelihood-of-confusion and dilution tests for a
   reputed mark cut against a single-letter prefix swap. Keeping the name
   through a public release would invite, at minimum, a takedown or
   cease-and-desist; renaming before publication eliminates that exposure
   cleanly. The code itself, the UX inspiration, and the GPL license carry
   no comparable risk (cf. CJEU C-406/10, *SAS Institute v. World
   Programming*, on the non-copyrightability of software functionality and
   look-and-feel) — the name is the single structural change required.

Given the quality bar the maintainer is holding the codebase to, the product
deserves a name worth being proud of, and one that does not pick a fight with
Apple's legal team. A better name must be chosen before the first public
release.

Current candidate names the maintainer likes: **Needle** and **Spindle**. Both
evoke physical-media / turntable imagery, which fits the product's dense,
pre-streaming, library-ownership ethos. Neither has been committed to yet, and
each needs a search-collision and trademark check (existing audio software,
bands, products) before being adopted.

Direction worth exploring when the time comes:

- short, pronounceable, and easy to say out loud
- not a play on `iTunes`, `Rhythmbox`, or any other existing player
- not `x`-prefixed and not leaning on dated Linux-desktop naming conventions
- music-adjacent without being literal (oblique nouns from records, playback,
  or musical structure tend to age better than compound words)
- searchable: distinct enough that the project is findable without colliding
  with an existing product, band, or common word
- usable as a binary name, crate prefix, and reverse-DNS application id
  without awkward transformations

Renaming touches the binary name, crate prefix (`sustain-*` / `sustain_*`), the
application id (`io.github.open_sustain.sustain`), packaging metadata, and every
SPDX/copyright header. Plan the rename as a single coordinated change rather
than a drip of partial renames.

## Summary

developer => desired contextual menu on tracks:
- Add to playlist => list
- Play Next (inject next in the queue)
- Get Info (show the full edition window, multitab)
- Show Album (switches to the associated Album view)
- Copy (actually copy the audio file itself)
- Show in folder (open nautilus or whatever)
- Remove from library (already implemented)
- Move to trash (already implemented)

`Sustain` is a Linux-only, Debian-first music library/player built with Rust,
GTK4, GStreamer, and SQLite. The product target is an iTunes 8~12-like desktop
music manager centered on a dense table/list workflow.

Rhythmbox is an import source only. The application owns its database, playlists,
statistics, search behavior, settings, and playback state. Ratings are part of
the application model, but their durable storage is the audio file's native
metadata tags.

## Architecture

Use a layered Rust workspace. GTK4 and GStreamer are edge adapters, not the
center of the application.

```text
crates/
  domain/         Pure music-library model and application vocabulary
  library_store/  SQLite persistence, schema, and migrations
  metadata/       Audio tag reading/writing and artwork extraction
  playback/       GStreamer playback controller
  importer/       Rhythmbox import pipeline
  search/         Query, filtering, sorting, and indexing behavior
  settings/       User preferences, starting with the library folder
  desktop/        MPRIS, media keys, and D-Bus integration
  ui_gtk/         GTK4 interface
  app_runtime/    Application wiring, commands, state, and background tasks
```

Dependency direction:

```text
ui_gtk        -> app_runtime -> domain
desktop       -> app_runtime -> domain
playback      -> domain
library_store -> domain
metadata      -> domain
search        -> domain
importer      -> domain + library_store
settings      -> domain
domain        -> no application-specific dependencies
```

The `domain` crate must not depend on GTK, GStreamer, SQLite, D-Bus, filesystem
watchers, or Rhythmbox-specific formats.

## Core Model

Use precise, stable naming. Prefer `Track` over `Song`.

Core vocabulary:

- `Track`
- `TrackId`
- `Playlist`
- `PlaylistId`
- `PlaylistEntry`
- `SmartPlaylist`
- `SmartPlaylistRule`
- `Rating`
- `PlayStatistics`
- `LibraryQuery`
- `TrackSort`
- `PlaybackState`
- `PlaybackCommand`
- `MetadataChange`
- `UserSettings`

SQLite is the canonical library index after import. File paths should not be
treated as permanent track identity.

Ratings must be written to the audio file's native tag format:

- MP3 uses ID3
- Ogg and FLAC use Vorbis comments
- MP4/M4A uses MP4 metadata atoms

SQLite may cache ratings for fast table rendering, search, and sorting, but it
must not become the only durable rating store.

The first schema should cover:

- tracks
- track locations
- playlists
- smart playlists
- smart playlist rules
- playlist entries
- metadata
- ratings
- play statistics
- settings
- schema migrations

## Library Management Roadmap

Start with a non-destructive library model.

In the first stage, scanning a configured library folder should index the files
where they already are. Sustain must not rename, move, copy, or reorganize files
unless the user explicitly enables a future managed-library setting. Manual file
addition should also be accepted without forcing those files into a specific
filesystem layout.

Keep the model compatible with a later `Keep my library organized` setting. When
enabled, that mode should copy added/imported files into clean subfolders under
the configured library folder. The exact naming template is still to be
confirmed, but the intended shape is close to:

```text
Artist/Album/NN Title.ext
```

The fuller candidate pattern is:

```text
$if2(%albumartist%,%artist%)/$if($ne(%albumartist%,),%album%/,)$if($gt(%totaldiscs%,1),%discnumber%-,)$if($ne(%albumartist%,),$num(%tracknumber%,2) ,)$if(%_multiartist%,%artist% - ,)%title%
```

Do not implement managed-library organization in the first stage, but avoid
scan and database assumptions that would make it painful later. In particular:

- track identity must remain stable and must not be derived only from the file
  path
- the database should be able to distinguish the track from its current file
  location
- rescans must update existing tracks instead of blindly appending duplicates
- missing files should remain visible in the library instead of disappearing
  silently
- the scanner should report added, updated, missing, skipped, and failed files
  as explicit outcomes

When a file recorded in the database no longer exists on disk, table views
should show a warning symbol on that row. Attempting to play the missing track
should offer a `Locate` workflow that lets the user choose the replacement file
manually. Relocating should update the track location while preserving playlists,
rating, metadata cache, and listening statistics.

## Library Scan Performance (Known Issue)

Measurement context: the observations below were taken from an unoptimized
debug build (`cargo run -p sustain-app`, `dev` profile, `unoptimized +
debuginfo`). A release build is expected to be substantially faster on the
CPU-bound phases (tag decoding, hashing, SQLite work), so the absolute
numbers will move. The structural problem — synchronous work on the GTK
main thread freezing the UI — is not a profile-level issue and will still
be present in release; the hard requirements below must hold for both
profiles, with the validation target re-measured on a release build before
this is considered closed.

The current scan path does not survive a real-world library. Observed behavior
on a ~10,000-track library:

- the first 2–3 seconds appear to enumerate files from the filesystem
- after enumeration, the UI freezes completely with one thread pinned at 100%
- the freeze persisted for at least 45 seconds before the maintainer killed the
  process
- after restart, the status bar showed the correct counts/durations, meaning
  scan progress was being written to SQLite throughout — the work was
  succeeding, but on a thread that should not have been the UI thread

This is unacceptable. The UI must never freeze, regardless of library size,
and a 10k-track library is small, not large. Scan responsiveness and
throughput are a first-class concern, not a polish item.

Hard requirements:

- the GTK main thread must never block on scanning work; no synchronous
  filesystem traversal, no synchronous tag reads, no synchronous SQLite
  writes triggered from the UI thread
- the scan runs entirely as a background task, communicating with the runtime
  via typed messages
- the status bar surfaces live progress (files seen, files indexed, current
  phase) with the rotating sync icon, and the table remains fully interactive
  while a scan is in flight
- cancelling a scan is supported and prompt; the partial result already
  written to SQLite is preserved and consistent
- a scan can be interrupted by app exit at any point without corrupting the
  library database

Design directions to evaluate before committing to an implementation:

- split the scan into explicit phases (enumerate, stat, tag-read, hash if
  needed, persist) so each phase can be tuned and instrumented independently
- parallelize the CPU/IO-bound phases across a worker pool sized to the host
  (likely `num_cpus`-based, with a cap), keeping SQLite writes funneled
  through a single writer to avoid lock contention
- batch SQLite writes into transactions of a tuned size rather than one
  statement per track; measure the sweet spot
- use `WAL` journaling mode for the library database so readers (the UI) are
  not blocked by the scan writer
- prefer streaming the enumeration into the worker pool rather than
  collecting the full file list first, so work begins overlapping with
  enumeration
- consider an explicit bounded channel between phases so a slow phase
  applies backpressure instead of memory ballooning
- measure before optimizing: add lightweight per-phase timing/counters that
  can be inspected (debug log or a hidden diagnostics view) so future
  regressions are catchable

Non-goals for this work:

- no premature caching layers, no custom thread pools where `rayon` or a
  scoped worker pool would do, no bespoke async runtime
- no hand-rolled SQLite connection pool before measuring whether a single
  writer thread plus read-only connections suffices

Validation target: on the maintainer's ~10k-track library, a full cold scan
must complete without ever blocking the UI for more than a single animation
frame, and a warm rescan (no changes) must be substantially faster than a
cold scan because unchanged files are detected cheaply (mtime + size, not
full tag re-read).

## Table Interaction Performance (Known Issue)

Measurement context: the observations below were taken from an unoptimized
debug build (`cargo run -p sustain-app`, `dev` profile, `unoptimized +
debuginfo`). Debug builds penalize per-row bind work, sort/filter passes,
and any hot path that crosses generics or iterator chains, so the absolute
sluggishness will partially shrink under `--release`. However, the
Rhythmbox baseline cited below is also a distribution build, not a debug
build, so the comparison is fair only after we re-measure Sustain in
release. Before declaring a regression vs. Rhythmbox, the table targets
must be re-validated on a release build; structural issues uncovered along
the way (non-virtualized rows, per-bind allocations, synchronous queries
on selection) are profile-independent and must be fixed regardless.

Independently from the scan freeze, the Songs table itself does not stay
responsive at real library sizes. Observed on the maintainer's ~10k-track
library, on top-tier consumer hardware as of mid-2026:

- scrolling is visibly sluggish, not 60+ fps
- a single click to activate a row has a roughly 500 ms lag before the row
  reflects the new selection
- the rest of the UI feels heavy in proportion

This is unacceptable. The target is fluid, instantaneous interaction at the
scale of a real library, not at the scale of the in-memory fake fixtures used
during interface-first development. 10k tracks is small; the table must
remain crisp at that size and degrade gracefully well beyond it.

Reference point: the exact same ~10k-track library is perfectly fluid in
Rhythmbox on the same hardware. Scrolling is smooth and row selection is
instantaneous. The performance target here is therefore known to be
achievable on this machine with this dataset — this is not chasing a wild
goose, it is matching a baseline that an existing GTK music player already
hits. If Sustain cannot match it, the bottleneck is in our own code, not in
GTK, the library, or the hardware.

Hard requirements:

- scrolling stays at the display's native refresh rate with no dropped frames
  on a populated table
- single-click row activation reflects in the UI within one frame; no
  perceptible lag between input and visual feedback
- sort, search/filter, and column resize stay interactive at 10k+ rows
- nothing in the row activation path performs synchronous SQLite queries,
  synchronous tag reads, or synchronous artwork decoding

Design directions to evaluate before committing to an implementation:

- confirm the table is using a virtualized GTK view (`GtkColumnView` /
  `GtkListView` with a `GtkListItemFactory`) rather than materializing a
  widget per row; the lag pattern strongly suggests the row set is not
  virtualized or the factory is doing too much work per bind
- audit the per-row bind path for unnecessary allocations, string
  formatting, or property lookups that run on every scroll tick; precompute
  display strings once and store them on the view model
- ensure the underlying list model is a flat, indexable structure
  (`GListModel`-backed) with O(1) item access and O(log n) or better
  filtering/sorting via `GtkFilterListModel` / `GtkSortListModel`, not a
  hand-rolled linear filter that re-scans on every change
- decouple selection-change handling from any downstream work that is not
  strictly required to draw the new selection state; defer artwork loads,
  detail-pane updates, and metadata queries to idle callbacks
- batch model invalidations: a single visible change should trigger one
  redraw, not a cascade of per-row notifications
- measure with `GTK_DEBUG=interactive` and frame timing before guessing;
  the 500 ms click lag is large enough that it points at a specific
  blocking call, not at general overhead

Non-goals:

- no custom virtualization layer; GTK4 already provides one and the task is
  to use it correctly, not replace it
- no premature caching of derived view data before profiling identifies
  what is actually slow

Validation target: on the maintainer's ~10k-track library, the Songs table
scrolls smoothly with no perceptible jank, click-to-select is visually
instantaneous, and search/sort/filter updates apply within one or two
frames. These targets are gating for any claim that the interface-first
shell is "done"; meeting them only on small fake fixtures does not count.

## Gapless Track-to-Track Playback (Known Issue)

The current auto-advance path inserts an audible silence between tracks.
When the currently playing track hits end-of-stream, GStreamer emits an EOS
message on the playbin's bus; Sustain responds by stopping the pipeline,
loading the next track's URI, and restarting playback. Between "stop" and
"playing", the audio output is silent — observable as roughly a half-second
gap between consecutive tracks.

A half-second of silence between tracks is not acceptable in a music
player. Pauses between tracks break album playback, ruin transitions on
mixed records (live recordings, DJ sets, concept albums with crossfaded
tracks), and make every playlist feel disjointed. This must be fixed
before Sustain is shippable as a real player. It is deferred, not
forgotten.

The intended fix uses GStreamer's `playbin` `about-to-finish` signal
instead of the EOS bus message. `about-to-finish` fires shortly before the
current track ends, giving the application a chance to set the next
track's URI on the same `playbin` element. The pipeline then transitions
to the new stream without tearing down and rebuilding the audio path,
eliminating the silence. The current EOS bus watch is kept as a fallback
for true end-of-queue conditions (no next URI to hand off to), where
stopping the pipeline is the correct behavior.

Design directions to evaluate before committing to an implementation:

- the `about-to-finish` handler runs on a GStreamer streaming thread, not
  on the GTK main thread; it must resolve the next URI without touching
  `Rc<RefCell<...>>` state from the UI side, because those types are not
  thread-safe
- pre-compute the next track URI on the main thread (whenever the queue,
  shuffle, or repeat state changes) and store it behind an
  `Arc<Mutex<Option<NextTrack>>>` or equivalent; the streaming thread
  reads from that handle without blocking
- `stream-start` on the bus becomes the authoritative "now playing
  changed" event; play-count, last-played, and the now-playing UI all
  update from that message rather than from the about-to-finish signal,
  so they reflect the moment the audio actually switches and not the
  moment the next URI was queued
- skip handling stays on the existing manual-skip path; manual skip
  remains a `play_track` call that tears down and rebuilds, gapless
  handoff is only for natural track completion
- validate against the real library: gapless handoff must work across
  format boundaries (MP3 → FLAC, FLAC → MP3, AAC → Vorbis, etc.) and
  must not introduce clicks, pops, or sample-rate-mismatch artefacts;
  test fixtures alone do not catch these
- the existing EOS bus watch must remain the path for true end-of-queue
  (no next track) so the pipeline still stops cleanly and the UI state
  stops lying about what is playing

Hard requirements once this work begins:

- consecutive tracks in a playlist, album, or library queue play with no
  perceptible silence between them
- the play-count and last-played updates for the outgoing track happen
  at the correct moment, neither too early (before the audio actually
  finished) nor too late (after the incoming track has already started)
- shuffle and repeat modes work the same way with gapless handoff as
  they do today with the EOS path
- the existing bus-watch fallback for true end-of-queue continues to
  work and stops the pipeline cleanly

Non-goals:

- no custom audio mixer and no crossfade behavior; gapless means zero
  gap, not blended overlap
- no rewrite of the GStreamer abstraction; this is a refinement of how
  the existing `playbin` element is driven, not a replacement
- no preemptive decoding of the next track via a separate pipeline; the
  `about-to-finish` mechanism on a single `playbin` handles this without
  application-level orchestration

## Duplicate Consolidation

Users must be able to merge multiple library entries that represent the same
recording into a single canonical track. This is a manual, user-driven
operation, not an automatic deduplicator.

Selection and entry point:

- the Songs table supports standard multi-selection with shift-click and
  ctrl-click
- the row context menu exposes a `Consolidate to single track` action when two
  or more tracks are selected
- the action opens a consolidation dialog; it never proceeds silently

Consolidation dialog:

- lists every selected track with enough columns to disambiguate them
  (location, format, bitrate, duration, size, rating, play count, date added)
- the user picks, independently, the reference track for:
  - audio file
  - metadata (title, artist, album, genre, year, track/disc number, etc.)
  - artwork
- the dialog previews the resulting consolidated track before the user confirms

Merge rules:

- the surviving track keeps the chosen reference audio file as its location
- the surviving track's metadata is taken from the chosen metadata reference
  and written through the normal tag-writing path so the file on disk matches
- the surviving track's artwork is taken from the chosen artwork reference and
  written through the normal artwork path
- play counts across all selected tracks are summed into the surviving track
- last played is the most recent value across the selected tracks
- date added is the oldest value across the selected tracks
- rating is taken from the highest-rated selected track; ties prefer the
  metadata reference
- playlist memberships from the removed tracks are rewritten to point at the
  surviving track, preserving order and de-duplicating consecutive entries

Atomicity and safety:

- the operation runs as a single transactional unit at the domain level: tag
  writes, artwork writes, SQLite updates, playlist rewrites, and file deletions
  either all succeed or the library is left in its pre-consolidation state
- non-reference audio files are removed from disk and from the database only
  after the surviving track has been fully written and verified
- any failure during tag write, artwork write, or persistence aborts the
  operation and surfaces an explicit error; partially merged state is not left
  behind
- the operation is reported through the standard background-task status
  channel, with progress and outcome surfaced in the status bar

This feature lives in the domain/app_runtime layer and is exercised by the
GTK shell. The domain logic must be unit-testable without GTK, GStreamer, or a
real filesystem mount.

## Application Flow

Use command/query separation.

Commands include:

- play selected track
- pause/resume/stop
- seek
- set volume
- set rating
- create, rename, and delete playlist
- create, rename, update, and delete smart playlist
- add/remove/reorder playlist entries
- update metadata
- update settings

Queries include:

- list tracks with filter and sort
- list playlists
- list smart playlists
- get track details
- search tracks
- get play statistics
- get current playback state
- get settings

GTK should call commands and observe application state. GTK widgets must not
write directly to SQLite, GStreamer, or metadata files.

Keep `ui_gtk` split by durable interface sections instead of letting `lib.rs`
absorb the application. The GTK crate can own GTK-specific view models and
callbacks, but large widgets should live in focused modules such as
`preferences`, `track_table`, `top_bar`, `mode_bar`, `status_bar`, and
`playlist_sidebar`. Each module should expose small constructors and typed
callbacks rather than reaching across the UI tree directly.

## UI Direction

The main table/list view is the core product. Build this first.

Use an interface-first implementation strategy. The UI should be a real GTK
application shell, not a disposable static mock. It should use real widgets,
real layout, real commands, and real view-state types, backed initially by fake
in-memory library/playback/settings data. This lets the product iterate on its
primary differentiator, the interface, while keeping backend services swappable
behind the same application contracts.

Do not build throwaway mock screens that will need to be ripped out later. The
mocked phase should establish the permanent UI architecture:

- typed view models for visible tracks, playlists, playback state, settings, and
  empty states
- typed commands for search, selection, sorting, rating edits, playback controls,
  playlist actions, and settings updates
- fake services that return realistic library data and playback state
- GTK widgets that only talk to the runtime through commands/state observation

Backend wiring should replace fake services incrementally without changing the
UI contract.

Initial layout:

```text
MainWindow
  IntegratedTopBar
    Playback controls
    Volume control
    Now playing area
    Search field
    Window controls
  ContentArea
    PlaylistSidebar, visible only in Playlists mode
      Playlists
    MainContent
      ModeBar
        Songs
        Albums
        Playlists
      ContentStack
        Songs: full-width track table
        Albums: full-width album-cover grid
        Playlists: right-side track table
  StatusBar
    Track count
    Total duration
    Selection summary
    Background task status, bottom right
```

Avoid a separate empty window titlebar. The app owns its top chrome, and the
playlist sidebar stays below it, left of the main content area.

The integrated top bar should be about 50% taller than default GTK chrome, with
its controls sized up to match. The search field keeps normal GTK height and is
vertically centered. Its content should have moderate lateral padding without
moving the headerbar background or window controls.

Playback controls should appear as plain enlarged vector icons, not text buttons
or framed buttons.

The mode bar should have a subtle theme-aware background tint that works in
light and dark modes.

The playlist sidebar should be visibly darker than adjacent content in both
light and dark modes.

Light and dark appearance should follow native GTK/system theme behavior. Do
not add an Sustain theme picker. Keep theme-aware CSS tokens centralized and
make every custom tint, row state, control, and window surface work in both
native light and native dark modes.

The interface-first shell should cover the everyday core workflow before deep
backend wiring:

- realistic Songs table with dense rows and sortable headers
- table columns for Track Name, Artist, Album, Genre, Year, BPM, Bitrate, Type,
  Duration, Rating, Plays, Last Played, Date Added, and Track Number
- click table headers to sort by that column
- right-click table headers to show a column chooser with tickable visibility
  entries
- allow column reordering from the table header
- playlist sidebar with enough fake playlists to exercise scrolling and resizing
- Playlists mode with right-side table content
- search field filtering visible fake data
- rating display and editing
- playback controls, current-track display, pause/play state, and volume state
- status bar counts, durations, and selection summary
- status bar background task feedback, especially scanning status in the bottom
  right with a rotating sync icon
- settings dialog shell, especially library path and manual scan
- native light and dark theme behavior
- keyboard behavior, especially spacebar play/pause
- empty and populated library states

Album view can remain rough during this phase. Advanced browsing views,
visualizers, streaming, device sync, cloud sync, folder sync, and multi-machine
sync are out of scope.

When album view becomes real, artwork should influence the track dropdown
surface. For each album, detect the two dominant artwork colors. Use the main
dominant color for the expanded track container and choose text/accent colors
with enough contrast, derived from or compatible with the artwork palette. This
must work in native light and dark modes without becoming unreadable.

The same dominant-color detection is reused by the integrated top bar's
now-playing artwork. Non-square cover art currently leaves gray gutters around
the image; those gutters should be filled with the artwork's dominant color so
the small cover blends into the surrounding control surface. Detection happens
once per track when artwork is loaded and is cached alongside the artwork so
album view and now-playing share a single source of truth.

Clicking the small now-playing artwork zooms it into a large modal overlay
centered on the window, with a close affordance in the top-right corner.
Clicking the zoomed artwork itself flips the surface to show the track's
lyrics in the same frame; the close affordance remains visible on both faces,
and a second click flips back to the artwork. Both faces share the dominant
color as their background so the flip animation reads as one continuous
surface. The lyrics provider and lyrics-storage path are a prerequisite for
this feature and need their own design decision before this work can ship.

The now-playing area includes a thin horizontal progress bar showing playback
position within the current track. The bar is intentionally thin and must
stay that way visually; it should not be made taller to feel clickable.
Instead, the interaction behavior should make it feel generous without
changing its appearance:

- the hit area is roughly double the bar's visual height, extending upward
  so the user can click slightly above the bar and still seek; the lower
  edge of the bar stays aligned to its visual position
- while the cursor hovers anywhere over the now-playing container, a
  vertical indicator (a thin tick) protrudes above the progress bar at the
  cursor's horizontal position, previewing where a click would seek to;
  the indicator disappears when the pointer leaves the container
- a click anywhere in the hit area seeks immediately to the clicked
  position; press-and-drag continues to seek as the cursor moves, so the
  user can scrub through the track without releasing the button; release
  commits the final position
- during drag, the audio seek can be live (scrubbing) or deferred to
  release; this needs to be decided based on GStreamer seek cost on real
  files, but the visual indicator must track the cursor at native refresh
  rate either way
- the seek path goes through the existing playback command, not a direct
  pipeline call from the widget

Smart playlists are in scope after regular playlist and table behavior is
stable. They should be rule-based saved queries over the local library, not a
cloud/recommendation feature.

Playback controls should be wired through the real command path:

- previous track
- play/pause
- next track
- volume

The volume control must avoid accidental software amplification near the top of
the range. Values from `95%` through `99.9%` should be magnetized to `100%` or
back below the danger range so the user cannot casually leave the player at an
almost-maximum software volume. This protects a clean listening chain from
unintentional non-unity gain behavior.

## Later Statistics Views

Add one or two library statistics views after the core table, playlist, search,
rating, playback, settings, and import workflows are solid.

The first statistics views should stay practical and library-focused:

- distribution of tracks by genre
- distribution of tracks by bitrate range

Initial bitrate ranges:

- `<= 128 kbps`
- `> 128 kbps and < 256 kbps`
- `>= 256 kbps and <= 320 kbps`
- `> 320 kbps`

Treat these as diagnostic library views, not social, recommendation, cloud, or
listening-insight features.

## Runtime

Run GTK on the main thread. Move slow work to background tasks:

- Rhythmbox import
- metadata scanning
- search indexing, if needed
- artwork extraction
- filesystem watching

Use typed messages between background tasks and the application runtime. Avoid
shared mutable UI state.

Background task state is application state, not preferences-window state. Manual
scan can be launched from the settings dialog, but its progress and completion
feedback should be surfaced in the bottom status bar, bottom right, with a
rotating sync icon while active.

Wrap GStreamer behind a playback service so no other crate depends on pipeline
details.

Minimum playback surface:

```rust
pub trait PlaybackService {
    fn play_track(&self, source: TrackPlaybackSource);
    fn pause(&self);
    fn resume(&self);
    fn stop(&self);
    fn seek(&self, position: std::time::Duration);
    fn state(&self) -> PlaybackState;
}
```

## First Vertical Slice

Build the first version in this order:

1. Create the Rust workspace and crate boundaries.
2. Define domain types and application commands/queries.
3. Build the real GTK interface shell with fake in-memory data.
4. Render the main dense Songs table/list view.
5. Render Playlists mode with a resizable sidebar and right-side table.
6. Add search, filtering, sorting, selection, and status summaries over fake data.
7. Add rating display/editing over fake data through the real command path.
8. Add playback controls/state over a fake playback service.
9. Add the settings shell, library path, manual scan action, and native light/dark validation.
10. Add SQLite schema and explicit migrations behind the runtime contracts.
11. Import Rhythmbox tracks, playlists, and ratings.
12. Play the selected track through GStreamer.
13. Persist rating edits through metadata tags.
14. Persist settings.

This slice should prove the full architecture without adding non-core features.

## Waveform Analysis (Tentative)

Per-track waveform analysis would be a nice-to-have. Computing a downsampled
amplitude envelope (peak/RMS bins) per track at import or first-play time,
cached alongside the track in the library, would unlock a visual representation
of the audio that could be displayed somewhere in the UI.

The placement is undecided. Candidates worth considering later include the
now-playing area, a seek-bar replacement, or a track-detail surface. None of
these are committed; the right home has to be found before this is worth
implementing.

Constraints if/when this is built:

- analysis is a background task, never blocking the UI or playback start
- the cached envelope is small, fixed-resolution, and detached from the audio
  file itself so it survives metadata edits
- the renderer must work cleanly in both native light and dark modes

## Metadata Backfill (Tentative)

Two related opt-in actions, surfaced from the settings pane:

- `Fetch missing artwork` — for every track in the library that has no embedded
  cover art and no cached artwork, attempt to retrieve a cover image from an
  online source and write it to the track.
- `Fetch missing tags` — for every track with empty fields in its native tag
  format (ID3 for MP3, Vorbis comments for Ogg/FLAC, MP4 atoms for M4A),
  attempt to identify the recording from an online source and fill in only the
  fields that are currently missing.

Strict non-destructive contract, applies to both actions:

- existing artwork is never replaced, re-encoded, or re-cropped; tracks that
  already have any embedded cover art are skipped entirely
- existing tag fields are never overwritten, normalized, recased, or
  reformatted; only fields whose current value is empty/absent are populated
- a partial existing tag set is respected: if a track has Artist and Title but
  no Album, only Album may be filled
- on any ambiguity (multiple plausible matches, low-confidence match), the
  field is left empty rather than guessed
- both actions write through the existing tag-writing path so the file on disk
  and the SQLite cache stay consistent

Open design questions to resolve before implementation:

- which metadata provider(s) to use (MusicBrainz is the obvious local-friendly
  candidate; Cover Art Archive for artwork; AcoustID/Chromaprint for
  fingerprint-based identification when tags are too sparse to match by text)
- match-confidence threshold and how the user is informed when a track was
  skipped because no confident match was found
- whether the user can run the actions on a selection (e.g. selected tracks in
  the Songs table) in addition to whole-library runs from settings
- rate limiting and offline behavior; both actions must degrade gracefully
  without network access
- caching of negative results, so repeated runs do not re-query the provider
  for tracks that were already determined to be unidentifiable
- whether a dry-run/preview mode is needed before writes happen

Constraints if/when this is built:

- runs as a background task surfaced in the bottom status bar, never blocking
  the UI or playback
- network requests are gated behind an explicit user-initiated action; Sustain
  must not silently phone home during normal library scanning
- provider client lives in its own crate (e.g. `metadata_remote`) so the core
  metadata crate stays offline and testable
- per-track outcomes (filled, skipped-no-match, skipped-already-present,
  failed) are reported as explicit results, consistent with the scanner's
  outcome reporting

This feature is out of scope for the first vertical slice. It should not be
attempted before the local metadata scan, tag-writing path, and background-task
status surface are solid.

## Smart Shuffle (Tentative)

In addition to a pure-random shuffle mode, Sustain should offer a `Smart Shuffle`
mode that picks the next track by similarity to the currently playing track
rather than uniformly at random. Pure random shuffle remains the default and
must stay available; smart shuffle is an opt-in alternative selectable from the
shuffle control.

The goal is a coherent listening session that drifts within a stylistic
neighborhood without becoming repetitive. The feature is local-only: no cloud
service, no recommendation API, no telemetry. All scoring runs against the
local library and local listening statistics.

Candidate similarity features to score against the seed (current) track:

- release year proximity (e.g. within a small window of the seed's year)
- date-added proximity (e.g. within roughly +/- 2 months of the seed's date
  added, capturing tracks ingested in the same listening era)
- identical or related genre
- shared artist or album-artist
- BPM proximity
- key/mode proximity if available from metadata
- rating proximity, biased toward equal-or-higher-rated tracks
- play-count band, to avoid pulling in tracks never listened to alongside the
  seed's usual companions
- co-occurrence in the same playlists as the seed
- duration band, to avoid jarring length jumps

Anti-repetition rules, applied as hard filters before scoring:

- exclude tracks played within the last hour
- exclude the seed track itself
- exclude tracks already played earlier in the current shuffle session
- exclude tracks marked as skipped recently (once skip tracking exists)

Open design questions to resolve before implementation:

- exact feature set and weighting; the list above is a starting point and needs
  refinement against real listening behavior
- whether to use a hand-tuned weighted scoring function or a learned model
  (e.g. a small random forest, logistic regression, or gradient-boosted trees)
  trained on the user's own play/skip history; a learned approach is only
  acceptable if it stays fully local, deterministic enough to debug, and cheap
  enough to retrain on-device
- how training labels are derived (full plays as positives, skips as negatives,
  with appropriate care for short tracks and intentional next-track presses)
- where retraining runs (background task on idle, never blocking playback)
- how the model and its inputs are persisted (alongside the library database,
  with a clear invalidation rule when the library or statistics change)
- how to explain the next-track choice to the user, at least in a debug surface,
  so the behavior is inspectable rather than opaque

Constraints if/when this is built:

- scoring and selection must run off the UI thread and must never block
  playback transitions
- the algorithm must degrade gracefully on libraries that lack rich metadata
  (missing year, missing genre, missing BPM); missing features reduce to a
  neutral contribution rather than disqualifying tracks
- pure random shuffle behavior must remain bit-for-bit unchanged when smart
  shuffle is not selected
- the implementation lives in the domain/app_runtime layer behind the existing
  playback command path; GTK only exposes the mode toggle

This feature is out of scope for the first vertical slice and should not be
attempted before regular playback, ratings, play statistics, and playlist
behavior are solid.

## Developer Isolation (CLI Data Paths)

Once Sustain is installed system-wide (e.g. via `.deb`), a developer working on
a branch out of a checkout must not risk colliding with or corrupting the
installed instance's data — in particular the user's real music library
database, their settings, and any cached artwork. The current behavior, where
both the installed binary and a `cargo run` build read and write the same
on-disk locations, is unsafe: a single bad migration or schema change in a
dev branch can trash years of curated library state.

The fix is to make the data paths explicit and overridable from the command
line. Two complementary flags:

- `--config <path>`: use the given TOML settings file instead of the default
  XDG config location. The file is created if it does not exist.
- `--database <path>`: use the given SQLite database file instead of the
  default XDG data location. The file is created if it does not exist.

Both flags accept absolute or relative paths and operate independently; a
developer can override one without the other.

For the common case (run a dev build in an isolated sandbox without typing
two paths every time), a single convenience flag:

- `--local-scope` (alternatively `--dev`): place both the TOML config and the
  SQLite database in the current working directory, under predictable names
  (e.g. `sustain.toml` and `sustain.sqlite`). Cached artwork and any other
  on-disk artefacts produced by the run go under a sibling directory in the
  same working directory.

Precedence rules:

- explicit `--config` / `--database` always win over `--local-scope`
- `--local-scope` wins over the default XDG locations
- nothing reads from the default XDG paths if any override flag is active;
  the dev instance must be fully self-contained in the overridden locations
- the resolved paths are logged at startup so the developer can see
  immediately which database and config the running instance is touching

Non-goals:

- no implicit "detect cargo run and switch modes" magic; the developer
  opts in by passing a flag
- no environment-variable equivalent in the first pass; flags are explicit
  and visible in shell history

Once this lands, the dev instructions in `README.md` must document the
flags and recommend `--local-scope` (or the explicit pair) as the standard
way to run Sustain against anything other than the installed user's real
library. The recommendation should be prominent enough that a new
contributor cannot miss it.

## Distribution

Target Debian as the primary distribution platform. The project should produce a
`.deb` package that installs cleanly on Debian stable and Ubuntu LTS without
requiring users to build from source or add third-party repositories.

Package deliverables:

- a `debian/` directory with standard packaging metadata (`control`, `rules`,
  `changelog`, `copyright`, etc.)
- correct dependency declarations for GTK4, GStreamer, and SQLite runtime
  libraries
- a desktop entry and icon installed to standard XDG locations
- a reproducible build via `dpkg-buildpackage` or `debuild`

Keep packaging simple and conventional. Use `debhelper` and `dh-cargo` (or
cargo invocation from `debian/rules`) as appropriate for a Rust project. Avoid
custom install scripts when standard `dh_install` suffices.

Ubuntu compatibility should follow naturally from targeting Debian, but do not
add Ubuntu-specific patches or PPA infrastructure in the first pass. A clean
Debian source package that builds on both is the goal.
