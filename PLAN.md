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

`Sustain` is a Linux-only, Debian-first music library/player built with Rust,
GTK4, GStreamer, and SQLite. The product target is an iTunes 8~12-like desktop
music manager centered on a dense table/list workflow.

Track context menu — all shipped:

- Add to playlist
- Play Next
- Get Info — multi-tab editor (see `## Get Info` for the two remaining
  enhancements)
- Show Album — switch to the associated Album view
- Copy — copy the audio file itself
- Show in folder — open Nautilus
- Remove from library
- Move to trash

Adjacent playback action still pending: an explicit **Add to Queue**
(append to tail), distinct from **Play Next** (insert at head). See
`## Up Next Queue`.

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

## Pre-Release Punch List

A tight index of the small, well-scoped open items that are not large
enough to warrant their own design section. Each links to the detailed
section where applicable; items without a detailed section are scoped
fully here.

- **Scan parallelization** — see `## Library Scan Performance`.
- **Live per-phase scan progress in the status bar** — see
  `## Library Scan Performance`.
- **Gapless track-to-track playback** — see
  `## Gapless Track-to-Track Playback`.
- **`Add to Queue` (append) action** distinct from `Play Next` — see
  `## Up Next Queue`.
- **Within-playlist drag reorder wiring**: `PlaybackCommand` for moving a
  playlist entry exists in the runtime; the regular-playlist track table
  must dispatch it on within-playlist row drags. No GTK-only row reorder
  path.
- **Now-playing consumes the shared artwork cache**: now-playing currently
  decodes artwork inline. It should consume the same cache that Albums
  view uses, so cache invalidations propagate to both surfaces with no
  per-surface refetch.
- **Get Info: track-to-track arrow navigation from the main window** —
  see `## Get Info`. Batch editing in Get Info is a separate, deferred
  product question.
- **Preferences module split** — `preferences.rs` is now large enough that
  the next non-trivial change should split it into per-section widget
  modules backed by a settings view model, in line with the broader
  split-when-touched pattern.
- **Settings window: tabbed navigation in place of the title bar** —
  the Settings dialog drops the conventional GTK title bar in favor of a
  tab strip at the top of the window that doubles as the window's drag
  surface. Initial tabs:
  - **Library** — library folder picker, managed-mode tickbox (see
    `## Library Management`), and the manual scan trigger.
  - **Analysis** — the BPM, Key, and Waveform detection tickboxes (see
    `## Audio Analysis and Consolidation Grouping`).
  No `APIs` tab is planned. The two networked features in the plan
  (`Fetch missing artwork`, `Fetch missing tags` — see
  `## Metadata Backfill`) work against MusicBrainz, Cover Art Archive,
  and AcoustID; the first two need no key at all, and AcoustID's
  per-application client key is an app-level secret embedded in the
  binary (as Picard does), not a value the user types. An `APIs` tab
  becomes justified only if/when Sustain grows a feature that requires
  per-user credentials — Last.fm scrobbling, Spotify "now playing", or
  any other account-bound integration — at which point it gets added
  alongside the feature that introduces it.
  The window height auto-sizes to the active tab's content rather than
  being fixed; switching tabs reflows the window to fit the new pane.
  Width stays stable across tabs so the chrome doesn't jitter
  horizontally on tab switches. This shape is also what the
  `Preferences module split` above should target — one widget module per
  tab, behind the shared settings view model.
- **BPM, Key, and Waveform analysis tickboxes in Settings** — three
  independent opt-in toggles so users can pick which heavy local-CPU
  pipelines they want. See `## Audio Analysis and Consolidation Grouping`.
- **Single-instance enforcement** — see
  `## Single-Instance Enforcement (Library Integrity)`.
- **Developer-isolation CLI flags** (`--config`, `--database`,
  `--local-scope`) — see `## Developer Isolation (CLI Data Paths)`.
- **Auto-dismiss + multi-feedback carousel in the status lane** — see
  `## Status Feedback Behavior (Desired)`. Needs a feasibility pass on
  GTK4 animation primitives and the message-queue ownership model
  before implementation.

## Get Info

The Get Info window is shipped: multi-tab editor, saving through
`ApplicationCommand::UpdateMetadata`, with deterministic context-action
availability. Two enhancements remain:

- **In-dialog Previous/Next navigation across tracks** — planned, see
  *Get Info: Track-to-Track Navigation* below.
- **Batch editing across a multi-track selection** — desired, deferred.
  Editing one field across multiple selected tracks (with explicit
  mixed-value handling per field) is product-tricky and not on the
  immediate path. The single-track edit model is the foundation; batch
  editing lands after the multi-track field-change model is explicit.

### Get Info: Track-to-Track Navigation

Match the iTunes 11 affordance: a pair of Previous/Next arrow buttons in the
bottom-left corner of the Get Info dialog walk a cursor over the displayed
track list while the dialog stays open. Edits are committed implicitly on
navigation — that is the explicit product decision, not the only option
considered. The dialog becomes a walkable inspector over the current view's
ordering rather than a strictly per-track modal.

**Goal.** A user inspecting/editing many tracks in sequence can stay inside
one dialog, hit Next, and continue editing the next track in the visible
order without re-opening the dialog.

**Cursor model.** At dialog open time the caller snapshots the current view's
ordered list of `TrackId`s and the index of the track the user invoked Get
Info on. The dialog owns that snapshot for its lifetime; it does not react to
the underlying filter/sort changing. This matches iTunes and avoids the
"cursor jumps because the live list reordered under you" surprise.

**Save semantics: implicit commit on navigate.**

- Clicking Previous/Next triggers the same commit pipeline as OK does today
  (metadata diff, rating diff, play-count reset), then loads the new track.
- Cancel still works on the currently visible track only — earlier
  arrow-committed edits stay committed. This is the iTunes-faithful
  behavior; the dialog title bar and an unobtrusive "saved" cue are out of
  scope but worth considering once shipped.
- If any dispatch fails (tag-write error, file gone, etc.), the dialog
  stays on the current track and surfaces the error — navigation does not
  swallow failures.

**Bounds and disable state.** Previous is disabled at index 0; Next at
`len-1`. No wrap-around. If a track in the snapshot has been removed from
the library by the time the cursor reaches it (background task, external
deletion), the dialog skips it transparently to the next valid neighbor in
the navigation direction; if none exists, the corresponding arrow disables.

**Window title.** Update the title from the static "Get Info" to include
the current track's display name, so the user always knows which row of the
snapshot they're editing. The title is the only chrome that changes on
navigate.

**Refactor scope.** The current dialog hard-codes "one track per instance":
`initial_metadata`, `initial_rating`, `track_id` are captured by value into
the OK closure and into the artwork add/remove closures. Generalizing this
requires:

- A shared `Rc<RefCell<DialogCursor>>` holding `{ ordered_ids, index,
  current_track, baseline_metadata, baseline_rating, baseline_play_count }`.
  Every closure that currently captures one of those values reads it through
  the cursor instead.
- Each page gains a `reload(track, baseline_metadata, artwork)` entry
  point:
  - `DetailsPage::reload` — repopulate every field, reset the rating-star
    state, reset the play-count reset-button armed flag.
  - `LyricsPage::reload` — replace the buffer text.
  - `ArtworkPage::reload` — replace the frame contents, re-arm the Remove
    button's sensitivity, swap the embedded-vs-missing note.
  - `file_page` — currently a free function returning a `Widget`. Promote
    to a struct with `reload(track, absolute_path)`.
  - `build_header` — either return label refs in the `Header` struct and
    mutate them on reload, or rebuild the row and swap it under the
    full-width tinted container.
- The diff-against-baseline contract becomes the single most important
  invariant: the baseline must be re-snapshotted from the *new* track's
  metadata every time the cursor advances. A stale baseline would leak
  edits from track A into track B's commit. Worth a focused test once the
  refactor lands.
- The caller (`track_context.rs` action wiring) computes the displayed
  ordered list of track IDs at the moment Get Info is invoked and hands it
  to `open_track_info_dialog` alongside the starting index. The dialog
  signature changes from `(track_id)` to `(track_ids, start_index)`. Other
  callers — if any — that don't have a meaningful surrounding list pass a
  single-element vec; the arrows stay disabled.

**Keyboard.** Bind `Ctrl+]` / `Ctrl+[` (iTunes's bindings) to Next /
Previous inside the dialog, alongside the buttons. `Esc` keeps its current
"close without saving the current track" meaning.

**Out of scope for this feature.** No schema changes, no new
`ApplicationCommand` variants — every commit path already exists. No
changes to the entry points other than the dialog signature. Batch editing
remains separately deferred.

**Risk surface.**

- Stale baseline → cross-track edit leakage. Mitigated by structuring the
  reload as "load track, snapshot baseline, then bind UI" and adding a
  test that opens the dialog, edits track A, navigates, asserts the
  in-flight commit was scoped to A.
- Removed-track-during-navigation race. Mitigated by the
  skip-to-next-valid rule above; if both directions exhaust, the dialog
  closes with a status message rather than dead-arrowing.
- Tag-write failure mid-walk. The existing OK error path stays the model:
  surface the error, keep the dialog open on the current track, do not
  advance the cursor.

**Estimated effort.** Medium refactor, ~200–400 LOC across `track_info.rs`,
the four page modules, and the single caller that opens the dialog. No new
dependencies. Cleanly removes the latent "one track per dialog instance"
assumption without leaving a hack surface.

## Schema Versioning and Migrations (Post-Release)

The first public release must ship with a versioned schema and a migration
runner. A `schema_version` row (or `PRAGMA user_version`) records the
applied version; on startup, the app applies any pending migrations in
order before opening the library.

Until the first release, this is **not** built: the schema is edited in
place and the maintainer wipes the local database. The versioning
infrastructure lands as part of the release-prep work, with version 1
defined as whatever schema ships in the first release. Every post-release
schema change adds a new numbered migration and bumps the version.

## Library Management

Sustain supports two library-management modes, surfaced as a single tickbox
under the library path in Preferences:

- **Don't touch my files** (default): scan the configured library folder and
  index the files where they already are. Sustain never renames, moves,
  copies, or reorganizes files. Manual additions are accepted without forcing
  a specific filesystem layout.
- **Keep my library organized**: Sustain owns the library layout. Newly added
  files are copied into the managed artist/album/track layout; existing
  indexed files are reorganized in the background; every canonical track path
  stays relative to the configured library root; files outside the library
  root are never stored as canonical track locations.

There is no separate `Consolidate` / `Organize Existing Library` button.
Enabling the tickbox starts the background organization task; disabling it
during a run requests cooperative cancellation (the current file move
finishes, no further tracks are moved). Disabling later does not move files
back; it only changes behavior for future additions.

### Non-negotiable safety rules

These apply to managed mode and are not optional:

- track locations are library-relative only
- managed organization never copies in-library files to reorganize them; it
  performs metadata-only same-filesystem moves
- the move primitive must refuse to overwrite an existing destination
- cross-device moves fail rather than falling back to copy/delete
- missing tracks are skipped and reported
- path planning for the batch happens before touching files
- playlist membership, track IDs, ratings, metadata, and statistics are
  preserved by retargeting existing tracks rather than creating new ones
- no GTK widget implements its own file move/copy path
- scan, import, and organization tasks are mutually exclusive
- library path changes and manual scans are blocked while a library task is
  running

### Filesystem move shape

Rust `std::fs::rename` overwrites existing destinations on Unix, which is
unacceptable for user-owned audio files. The managed-mode move is therefore a
same-filesystem metadata move: require a regular source file, refuse an
existing destination, create the destination directory, hard-link the source
to the destination, then remove the old source. This copies no content,
spares the SSD, and gives atomic no-overwrite destination creation. If the
filesystem does not support hard links or the paths are on different devices,
the move fails — it never falls back to copy/delete.

### Recovery journal

Filesystem moves and SQLite updates are not one atomic transaction. Managed
organization writes a small journal in the library root before moving files,
containing the planned track id, source relative path, and destination
relative path for each move. On startup, and before a new organization task,
Sustain reconciles the journal:

- destination exists and source does not: update SQLite to the destination
- source and destination both exist as the same file: remove the old source
  and update SQLite to the destination
- source exists and destination does not: leave the track at the source
- source and destination both exist but are different files: delete nothing
  and leave the current database record untouched

The journal is removed only after the task finishes or cancels with all
completed moves reflected in SQLite.

### Path planner

The managed path is produced by a pure domain planner. First-pass layout:

```text
Artist/Album/NN Title.ext
```

Planner requirements:

- preserve the original extension
- never produce an absolute path
- never produce `..` components
- never produce empty components
- sanitize path separators and control characters
- handle missing metadata deterministically
- resolve collisions deterministically
- return structured plans

Open product decisions:

- exact artist and album fallback wording
- multi-disc filename style
- compilation handling
- user-editable path templates

The fuller pattern the maintainer is considering when templates land:

```text
$if2(%albumartist%,%artist%)/$if($ne(%albumartist%,),%album%/,)$if($gt(%totaldiscs%,1),%discnumber%-,)$if($ne(%albumartist%,),$num(%tracknumber%,2) ,)$if(%_multiartist%,%artist% - ,)%title%
```

### Duplicate detection on managed add

Managed add skips external files already present in the library by content
hash. Normal library scans must not hash file contents. During managed
import, existing tracks without hashes are checked lazily by file size
first, then hashed only when size matches an incoming file. Automatic
organization of existing files does not deduplicate tracks — it moves the
existing indexed track records to their managed paths while preserving
identity.

Path-affecting metadata edits also participate in managed organization. When
the user edits artist, album artist, composer, album, title, track number,
disc number, disc total, or compilation status while managed mode is
enabled, Sustain plans a new managed path and moves the existing file there
if the path changes.

### Drag-and-drop import

When `Keep my library organized` is active, the main Songs list accepts
drag-and-drop from the GNOME file manager as a first-class import path.
Dropping audio files copies each file into the managed library folder using
the active naming template and indexes it. After a successful copy, ask the
user whether to remove the original source files that lived outside the
managed library folder. Concrete details (folder handling, dedupe rules,
prompt defaults, behavior of the same gesture under `Don't touch my files`)
are decided at implementation time.

### Track identity and missing files

Track identity must remain stable and must not be derived only from the file
path. The database distinguishes a track from its current file location.
Rescans update existing tracks instead of blindly appending duplicates.
Missing files remain visible in the library instead of disappearing silently;
the scanner reports added, updated, missing, skipped, and failed files as
explicit outcomes.

When a file recorded in the database no longer exists on disk, table views
show a warning symbol on the row. Attempting to play a missing track offers
a `Locate` workflow that lets the user choose the replacement file
manually. Relocating updates the track location while preserving playlists,
rating, metadata cache, and listening statistics.

## Library Scan Performance

The scan is off the GTK main thread (background worker + channel), SQLite
writes are batched in transactions, the library connection runs in WAL
mode with `synchronous = NORMAL`, the user can cancel a running scan
from the status bar, and the status bar reports the final summary.
The structural freeze that motivated this section is resolved.
Remaining open work, in priority order:

- parallelize the CPU/IO-bound phases (tag decoding, hashing, stat) across a
  worker pool sized to the host (`num_cpus`-based, capped), keeping SQLite
  writes funneled through a single writer to avoid lock contention
- surface live per-phase progress in the status bar (files seen, files
  indexed, current phase) alongside the rotating sync icon — not only the
  final summary

The WAL pragma is set during `SqliteLibraryStore::from_connection`; until
the read path gains a second connection, all reads and writes still
serialize on the single `Mutex<Connection>`, so WAL's reader/writer
isolation does not yet manifest — but commits are cheaper and the
infrastructure is in place for the eventual reader connection split.

Cancellation is cooperative: each background task in the status-bar
lane (scan, import, organize) carries an `Arc<AtomicBool>` flag that
its worker checks between files and between phases. The status bar
displays a single "Cancel" button next to the spinner whenever any
task is running, and clicking it dispatches
`request_background_task_cancellation` on the runtime — which sets
whichever of the three flags is currently live. While the worker is
winding down, the status label switches to "Cancelling..." and the
button disables; once the worker returns, the result flows through
the normal `apply_*` path and the summary reports `cancelled: true`.
Cancelled scans persist the partial set of tracks they did index and
deliberately skip the missing-tracks sweep — the unwalked portion of
the library is unknown, not missing. Cancelled imports roll back any
files copied so far so the filesystem is left exactly as it was
before the import started. The same pattern is where future
`Detect BPM` / `Detect key` / `Analyze waveform` runs will plug in:
their workers will share the cancel button, the `Cancelling...`
state, and the `cancelled: bool` summary contract.

Non-goals: no bespoke async runtime, no hand-rolled SQLite connection pool
before measuring whether a single writer thread plus read-only connections
suffices, no custom thread pools where `rayon` or a scoped worker pool
would do.

Validation target: on the maintainer's ~10k-track library (release build), a
full cold scan completes without blocking the UI for more than a single
animation frame, and a warm rescan (no changes) is substantially faster than
a cold scan because unchanged files are detected cheaply via mtime + size,
not full tag re-read.

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

## Up Next Queue

The queue model is in runtime: `PlaybackCommand::EnqueueNext` injects at the
head, the queue takes precedence over the implicit play order, and a
`Play Next` row context action dispatches it. Distinct from the iTunes 11
"Up Next" target in two ways that still need addressing:

- add an explicit `Add to Queue` action that appends to the tail (separate
  from `Play Next` which inserts at the head)
- expose a visible Up Next surface so the user can see what is queued; the
  UI placement (popover from the now-playing area, side panel, dedicated
  view) is undecided
- decide whether queue entries are reorderable and individually removable,
  and what those interactions look like (drag, context menu)
- decide whether queue state persists across app restarts

## Album-Centric Playback

`Play` and `Shuffle Play` triggered from an album (cover button, context
menu, double-click on the album tile) are scoped to that album only. The
play queue is loaded with exactly the tracks of that album — in disc/track
order for `Play`, in shuffled order for `Shuffle Play` — and nothing else.
When the last track of the album finishes, playback stops and the current
track is unloaded; the player does not fall back to the underlying library
order, does not auto-advance into a neighboring album, and does not pull in
any other tracks. Album playback is a self-contained session: it starts on
the album, stays on the album, and ends with the album.

## Duplicate Consolidation

Users must be able to merge multiple library entries that represent the same
recording into a single canonical track. This is a manual, user-driven
operation, not an automatic deduplicator.

Discovery — finding consolidation candidates:

In addition to ad-hoc multi-selection, Sustain should help the user *find*
likely duplicates. The discovery surface is a way *into* the consolidation
dialog described below, not a separate feature, and never merges anything
on its own.

- a `Show Duplicates` view groups tracks that share a loose normalized
  artist + title (case-insensitive, diacritic-folded, whitespace-collapsed)
- a stricter `Show Exact Duplicates` filter additionally requires matching
  album and duration (within a small tolerance) so distinct recordings of
  the same title don't get folded together
- groups of one are not shown; multi-track groups display adjacent so the
  user can select across the group and trigger consolidation
- the discovery view is read-only; every merge still goes through the
  consolidation dialog and explicit user confirmation

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

Longer-term, the Albums grid must be virtualized. The immediate performance
fix should remove eager startup work and move artwork extraction off the GTK
main thread, but it should preserve a clean album model/artwork-loader boundary
so the current eager box/grid can later be replaced by a GTK `GridView` /
factory-backed model without rewriting artwork loading or album grouping.

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

When the playing track has no embedded artwork and no cached artwork, the
now-playing tile renders an explicit "missing artwork" placeholder icon
(not a passive gray box) that signals to the user "click here to fetch a
cover for this track." This is a per-track, on-demand counterpart to the
bulk `Fetch missing artwork` action in `## Metadata Backfill`: same
underlying provider client, same non-destructive contract (never overwrite
existing artwork), same write path (embedded tag plus any artwork cache
update), but triggered inline on the track the user is hearing right now
rather than as a library-wide background sweep.

Interaction states for the small now-playing tile:

- **Missing** — no embedded or cached artwork. The tile shows the
  missing-artwork icon centered on the dominant-color background. The
  tile is clickable; hovering surfaces a tooltip that names the action
  (e.g. "Fetch artwork").
- **Fetching** — a click switches the tile to an indeterminate spinner
  on the same background. The spinner replaces the icon in place so the
  tile geometry does not jitter. The click is idempotent: further
  clicks while a fetch is in flight do nothing.
- **Resolved** — on successful retrieval, the artwork is written to the
  file's embedded tag through the standard tag-writing path, the
  artwork cache row for the track is updated, and the tile transitions
  to the normal artwork display (with dominant-color gutters as above).
  All other surfaces that show the same track's artwork — the zoomed
  overlay, the Albums grid tile, any track table cover column — pick up
  the update from the same cache invalidation, with no per-surface
  refetch logic.
- **Failed / no match** — the tile returns to the missing-artwork state
  and is clickable again for a manual retry. A brief, non-modal status
  bar message names the outcome ("No cover art found for this track").
  Failures are not cached as "permanently missing" — the user may have
  fixed connectivity, or a provider may have gained the cover since.

Constraints (mirroring the bulk path so the two surfaces stay coherent):

- the fetch runs on a background task; the GTK main thread is never
  blocked on a network request
- the network request is gated behind the explicit click — Sustain
  never silently fetches artwork while idling on a track
- if the playing track changes mid-fetch, the in-flight request is
  allowed to complete and its result is written to the originating
  track's file/cache; the now-playing tile only swaps to the new
  artwork if the originating track is still the playing track at
  completion time
- the same on-demand entry point should be reused on the zoomed-overlay
  artwork face and on the Albums grid tile when the album's
  representative track has no artwork, so the user can trigger a fetch
  from whichever surface they happen to be looking at; those secondary
  surfaces are nice-to-haves and can land after the now-playing tile

Write-while-playing safety: all tag writes (artwork, metadata,
ratings) go through `atomic_save_to_path` in the metadata crate, which
seeds a sibling temp file from the original, lets lofty rewrite the
temp's tag chunks, fsyncs, and then `rename(2)`s the temp over the
destination. On Linux/POSIX the kernel keeps any open file descriptor
pointing at the original inode until it is closed, so GStreamer's
in-flight playback reads continue against the unmodified bytes and the
new bytes only affect future opens.

Open question: whether the missing-artwork icon also belongs on Albums
grid tiles by default (always visible for art-less albums), or only on
the now-playing surface where the user is most likely focused. Settling
this depends on how cluttered the Albums grid feels with many missing
covers — a question that won't have a real answer until Albums view has
a sizable real library to look at.

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

Smart playlists are shipped as rule-based saved queries over the local
library (domain rules, runtime evaluation, sidebar rows, editor surface,
folder grouping). Relative-date evaluation runs against an injectable
`Clock` plumbed through `ApplicationRuntime`, so tests assert
date-window behavior deterministically against a fake clock.

A freshly created library is seeded once with five starter smart
playlists, gated on `SqliteLibraryStore::was_freshly_created()` (no
runtime flag, no meta state — schema creation is the trigger). The set
deliberately excludes iTunes entries that don't fit Sustain's
pure-local-music scope (no Music Videos, no Purchased, no
podcast/audiobook buckets) and avoids iTunes-specific vocabulary like
"Loved". Shipped defaults:

- **Recently Added** — `Date Added` in the last 14 days
- **Recently Played** — `Last Played` in the last 14 days
- **Top 25 Most Played** — `Plays > 0`, limited to 25 by Most Often
  Played
- **4+ Stars** — rating ≥ 4
- **Unplayed** — `Plays = 0`

The entries are ordinary user-editable smart playlists — they can be
renamed, modified, or deleted like any other, and a deletion sticks
(the seed never re-runs on an existing database). Exact rule shapes
remain open to refinement; the principle is that a freshly populated
library is never empty of useful default organisation.

## Keyboard Shortcuts

Sustain must be keyboard-driven. The reference is the iTunes shortcut set,
translated to Linux/GTK conventions (`Ctrl` instead of `Cmd`, GNOME-native
modifiers). The goal is not a literal port but a familiar muscle-memory
experience for ex-iTunes users on GNOME.

`Ctrl+L` Jump to Current Track is wired (raw key controller, see note
below). A broader shortcut pass for playback, navigation,
selection/editing, playlists, and window management still needs to be
specified — including conflict review against GNOME conventions and
in-app shortcut discoverability.

The work splits into two independent tracks that must not be conflated:

1. **Shortcut execution** — registering `gio::Action`s on the
   `gtk::Application` with the matching `set_accels_for_action` calls
   so each key combination invokes the right behavior anywhere in the
   window.
2. **Context-menu accel-label display** — surfacing each shortcut as
   muted right-aligned text on the matching context-menu entry. This
   requires the menus to be built from a `gio::Menu` model rendered
   through `GtkPopoverMenu`, which is a context-menu architecture
   migration in its own right (see "Context-menu accel-label display"
   below).

### Committed shortcuts (execution)

The following shortcuts are committed and must be wired through the GTK
action system (not hard-coded key handlers on individual widgets) so
they work globally across the window, surface uniformly in the GTK
shortcuts overlay, and can be inspected/overridden through standard
GNOME mechanisms.

| Action                                             | Shortcut         | Status   |
|----------------------------------------------------|------------------|----------|
| Create a new playlist                              | `Ctrl+N`         | Wired    |
| Create a new Smart Playlist                        | `Ctrl+Alt+N`     | Wired    |
| Focus the search field                             | `Ctrl+F`         | Wired    |
| Show the selected track's file in the file manager | `Ctrl+R`         | Wired    |
| Open the Get Info window for the selected track    | `Ctrl+I`         | Wired    |
| Jump to Current Track                              | `Ctrl+L`         | Wired*   |

\* `Ctrl+L` is currently bound through a raw `EventControllerKey` on the
window because it needs focus-aware bypass logic (don't intercept while
a text field has focus). Migrating it to a `gio::Action` requires
generalising that bypass through the action enabled-state, which is a
deliberate later cleanup and not part of the initial shortcut pass.

Behavior notes per shortcut:

- `Ctrl+N` creates an empty playlist with a unique default name,
  switches the main view to Playlists so the new entry is visible, and
  arms the inline rename so the next keystrokes type the playlist's
  name. The view switch is required: the sidebar is hidden in Songs and
  Albums modes, so arming a rename without it would silently lose the
  user's keystrokes.
- `Ctrl+Alt+N` opens the Smart Playlist editor with a unique default
  name pre-filled. Switching to Playlists view happens for the same
  reason as `Ctrl+N`; on save, the sidebar refreshes to show the new
  entry.
- `Ctrl+R` reuses the existing `Show in folder` context-menu action's
  underlying command (opens Nautilus, or the user's configured file
  manager via XDG). On a multi-row selection it acts on the first
  selected track — opening one window per track would be hostile, and a
  single predictable parent directory matches the muscle-memory the
  iTunes shortcut sets.
- `Ctrl+I` reuses the existing `Get Info` context-menu action. With
  the deferred multi-track batch-editing question (`## Get Info`),
  multi-selection behavior tracks whatever Get Info itself settles on,
  not a separate shortcut-level decision. The shortcut is a no-op when
  the selection is empty or larger than one row, matching the
  `Single`-selection contract the context-menu action already enforces.
- `Ctrl+F` focuses the existing top-bar search field and selects any
  current query so a fresh keystroke replaces it, matching standard
  GNOME find-bar behavior.
- A `Ctrl+Shift+N` "new playlist from the current selection" shortcut
  remains on the long-term wishlist but is not part of this pass: the
  selection-to-playlist flow is a feature in its own right (folder
  placement, scope across views) and must be designed before being
  bound to a key.

### Context-menu accel-label display

Every context-menu entry whose action has a committed keyboard shortcut
should show that shortcut as muted/secondary text aligned to the right
edge of the row (e.g. `Get Info        Ctrl+I`). This is the standard
`MenuItem` accel-label behavior and falls out for free when menu items
are built from `gio::Menu` entries pointing at registered actions —
that is the required path. Do not hand-format shortcut strings into
menu labels; the accelerator string must come from the action
registration so menu text, the shortcuts overlay, and the actual key
binding cannot drift apart.

The hint is muted (dim-label / secondary text colour) so the menu still
reads as a list of *actions*, with the shortcuts as ambient reference
rather than competing for attention. This must work in both native
light and dark modes.

This is **not done yet** and is deliberately decoupled from the
execution pass above. The current track context menu is a hand-built
`gtk::Popover` of `gtk::Button` rows because it carries selection-
sensitive enablement, a confirmation flow for trash, a dynamic nested
"Add to Playlist" submenu derived from playlist folders, and bounded
scrolled rendering for that submenu — all of which were stabilised
through fragile bug fixes. A `GtkPopoverMenu` migration is a context-
menu architecture migration, not a shortcut-wiring side quest. It must
be approached and tested as such, starting from a small proof on the
simpler entries (Get Info, Show in Folder, Remove) before tackling the
Add to Playlist submenu deliberately.

## Search Bar

The top-bar search field is wired into the live query path with a debounced
keystroke filter. The remaining design surface — which is product, not
infrastructure — is still open and should be settled with the maintainer
before extending the current behavior:

- **searched fields**: which track fields participate in the match?
  Candidates include title, artist, album artist, album, genre, year,
  composer, comment, and file path. Whether the user can scope the search
  to a specific field is also undecided.
- **match semantics**: substring vs. token-prefix vs. fuzzy; case
  sensitivity; diacritic folding; whether multiple whitespace-separated
  terms are ANDed across fields (typical library-search behavior) or
  treated as a single phrase.
- **operators and modalities**: whether to support quoted phrases,
  field-scoped queries (e.g. `artist:bowie`), negation (`-live`),
  numeric ranges (`year:1970-1979`), or rating filters (`rating>=4`),
  and how those interact with the smart-playlist rule grammar so the
  two systems share vocabulary where it makes sense.
- **behavior across modes**: in Albums mode, does the query filter the
  cover grid by any matching track, or only by album-level fields? In
  Playlists mode, does it filter within the selected playlist or
  across all playlists?
- **persistence**: whether the current query is preserved across app
  restarts or always starts empty.

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

## Status Feedback Behavior (Desired)

The bottom-right status surface that today carries scan progress and other
background-task state should evolve into a small notification lane with two
properties on top of what it already does. Both are desired features that
need a feasibility pass before implementation, particularly around GTK4
animation primitives and the exact threading/ownership model for a queue of
ephemeral messages.

- **Auto-dismiss for ephemeral messages.** Transient outcome messages
  (e.g. "Imported 12 tracks", "No cover art found for this track",
  "Saved playlist") fade out and disappear after a short timeout instead
  of persisting until the next message replaces them. Persistent
  in-progress states (the rotating sync icon while a scan is running,
  long-lived background tasks) do **not** auto-dismiss — they stay
  visible until the underlying task completes. The exact timeout is a
  product decision, not load-bearing on the design; somewhere in the
  3–6 second range is the starting point.
- **Multi-feedback carousel for concurrent operations.** When multiple
  operations finish close together, messages should not stomp on each
  other. Instead, they queue and animate horizontally — each new
  message slides in from the right, the previous one slides left and
  fades toward transparency, so the user can briefly read what just
  happened before the next message takes the foreground. The lane
  shows one message at a time at full opacity, with the outgoing one
  briefly visible mid-fade for continuity.

Constraints and open questions for the feasibility pass:

- the lane must not block the GTK main thread; animations should be
  driven by GTK's frame clock / `Adw` animation primitives, not by
  application-level timers spinning on the UI
- the message queue is application state, not widget state, so
  background tasks dispatch messages through the same command/state
  path as other runtime events; the widget only renders the current
  head of the queue
- persistent vs. ephemeral is a property of the message itself
  (background-task progress is persistent; one-shot outcomes are
  ephemeral), not of the widget
- if the queue grows unbounded under a burst (e.g. a bulk operation
  emitting one message per track), the lane should coalesce or
  collapse rather than animate through hundreds of messages — the
  exact policy (drop intermediate, replace-by-category, summary
  message) is to be settled with the maintainer
- accessibility: screen readers should observe each message as it
  appears regardless of the visual carousel timing

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
amplitude envelope (peak/RMS bins) per track, cached alongside the track in
the library, would unlock a visual representation of the audio that could
be displayed somewhere in the UI.

Enablement and pipeline shape are not separate from the other local-CPU
analyses: waveform analysis is one of the three independent tickboxes
under `## Audio Analysis and Consolidation Grouping` and runs through the
same opt-in, background-task, non-destructive contract as BPM and Key
detection. Waveform-specific notes:

- the cached envelope is small, fixed-resolution, and detached from the
  audio file itself so it survives metadata edits
- storage is the SQLite cache (not embedded tags) since waveform envelopes
  are not standard tag payloads
- the renderer must work cleanly in both native light and dark modes

UI placement is undecided. Candidates worth considering later include the
now-playing area, a seek-bar replacement, or a track-detail surface. None
of these are committed; the right home has to be found before the
rendering side is worth building. The analysis tickbox can ship before
the rendering surface exists — pre-computing envelopes for a library is
useful on its own and avoids a stall when a renderer is later added.

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
  fingerprint-based identification when tags are too sparse to match by text).
  None of these require a *user-supplied* key: MusicBrainz and Cover Art
  Archive are anonymous, and AcoustID's client key is an app-level secret
  shipped with Sustain (as Picard ships its own). No Settings credentials
  surface is needed for this feature on its own; that only becomes a
  question if a later feature introduces per-account integration.
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

Writes from this feature ride on the tag-writing path's atomic
replace-by-rename (see the write-while-playing note in `## UI
Direction`), so a backfill run against a track currently being played
does not introduce audio glitches.

This feature is out of scope for the first vertical slice. It should not be
attempted before the local metadata scan, tag-writing path, and background-task
status surface are solid.

## Audio Analysis and Consolidation Grouping

Three per-track derived values are in scope as local-CPU analysis features:

- **BPM detection** — analyze the audio to estimate beats per minute, write
  the result to the track's native tag and SQLite cache when no BPM is
  already present.
- **Musical key detection** — analyze the audio to estimate the key (e.g.
  Camelot or standard notation), persisted the same way.
- **Waveform analysis** — compute a downsampled amplitude envelope
  (peak/RMS bins) per track and cache it alongside the track in the
  library. Used by waveform-based UI surfaces (see
  `## Waveform Analysis (Tentative)`).

All three share the non-destructive contract of `Metadata Backfill` (never
overwrite an existing value) but differ in shape: they are local CPU work,
not network lookups, and they always produce a value (no "no-match-found"
outcome). Library and crate choice (`aubio`, `essentia`, pure-Rust
alternatives) is unresolved.

### Settings tickboxes

The Settings window exposes **three distinct tickboxes** — BPM analysis,
Key analysis, and Waveform analysis — not a single combined toggle. They
are independent analysis pipelines, can succeed or fail independently, and
the user must be able to enable any subset (e.g. enable BPM for Smart
Shuffle ordering without committing to key detection's heavier processing
or to waveform storage cost). The rationale is to insulate users from
heavy work they don't want: each pipeline has its own CPU footprint and
its own storage footprint, and each unlocks its own product surface.

Behavior when a tickbox is enabled:

- run the analysis as a background task over the library, scoped per the
  shared scope controls (whole library vs current selection vs
  missing-values-only)
- respect the non-destructive contract: only fill tracks whose target
  field/cache row is currently empty
- write through the existing tag-writing path (BPM, Key) or the dedicated
  waveform cache (Waveform) so the file on disk and the SQLite cache stay
  consistent
- surface per-track outcomes (filled, skipped-already-present, failed) and
  overall progress in the bottom-right status bar, like other background
  tasks

Disabling a tickbox cancels any in-flight run for that analysis and stops
future automatic runs. It does not remove already-written BPM/Key values
from tags, nor already-computed waveform envelopes from the cache.

### Grouping with metadata backfill

The five derived/fetched-data features share one settings surface so they
stay consistent and discoverable:

- `Fetch missing artwork` (network)
- `Fetch missing tags` (network)
- `Detect BPM` (local CPU)
- `Detect key` (local CPU)
- `Analyze waveform` (local CPU)

All five are opt-in, run as background tasks against the existing library,
respect the same non-destructive contract, and produce per-track outcomes.
They share one `Consolidation` tab in Settings with shared scope controls
(whole library vs current selection vs missing-values-only) and a unified
progress surface in the status bar. Settling naming, location, scope
controls, and background-task model once — before any of the five ship —
avoids ending up with five mismatched entry points scattered across the UI.

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

## Deferred Feature Backlog

The `README.md` feature list advertises a set of "_probably_ coming later"
capabilities. The ones with their own dedicated sections in this plan
(`## Duplicate Consolidation`, `## Metadata Backfill`, `## Smart Shuffle`)
are tracked there. The remainder is consolidated here so the plan covers
everything the README promises — none of these are first-vertical-slice
material, but they must not be lost.

### Library import (iTunes / Apple Music `.xml`, Rhythmbox)

Importers are explicitly deferred. The product-level rationale is that early
adopters can credibly use an LLM coding agent (Claude, Codex, etc.) to write
a one-off importer against their specific source library, so we must not
spend a significant slice of the first releases building polished import UIs.

When we do invest engineering time, the shape should be a **CLI importer
bundled with Sustain** rather than an in-app workflow — a GUI dialog can
come later once the CLI path is solid. Concrete delivery shape (subcommand
of `sustain` vs sibling binary, exact flags, etc.) is to be decided at
implementation time.

### Audio analysis (BPM detection, musical key detection)

The library schema and table view already expose a BPM column, and BPM is
cited as a Smart Shuffle input. What is missing is the **detection**
pipeline itself: an offline analysis pass that populates BPM and musical key
for tracks that lack them. Concrete shape (library/tool used, threading,
how results are surfaced) is to be decided at implementation time.

### Sync to Android / Export to external drive in Pioneer format (iPod-style device sync)

Sync a selected set of playlists from the library to an external device.
Two concrete targets:

- Android phones/tablets
- External drives (USB sticks, SD cards, SSDs) written in Pioneer's
  proprietary on-device format, consumable by Pioneer XDJ/CDJ hardware
  and by Rekordbox 5

Both are essentially the same workflow shape — the same one users used to
get from iTunes for iPod sync: a GUI panel to pick which playlists go to
the device, the player computes what needs to be written/removed, the user
confirms. This is GUI-driven, not CLI-first. Concrete shape (transport per
device, conflict handling, the panel UI itself) is to be decided at
implementation time.

One requirement is concrete already: Sustain must be able to reliably
**recognise devices it has seen before** and recall the set of playlists
that were ticked for each one, so the sync panel can come back up
pre-populated instead of starting empty every time.

The device itself only carries half of that information:

- A Pioneer-formatted drive carries the Pioneer `.pdb` database, which
  tells us what tracks/playlists currently live on the drive — but not
  which playlists the user had ticked in Sustain's sync panel for that
  drive.
- An Android device exposes the files that are present on it, with the
  same gap: we can see what's there, not what the user originally
  selected.

So the user-selection state has to live in Sustain, keyed by a stable
identifier for the device. The best/most solid options for that
identifier (USB serial, volume UUID, MTP device ID, fingerprint of the
on-device database, etc.) and the storage shape for the cache are to be
investigated at implementation time.

#### Pioneer-format reference implementation

The Pioneer-format export is not greenfield work. The maintainer already
ships a working Rust implementation of the on-drive format in
[`rhythmbox-to-pioneer-xdj-exporter`](https://github.com/AnnoyingTechnology/rhythmbox-to-pioneer-xdj-exporter),
and Sustain's external-drive export should be built by **lifting that
project's format-generation core into a Sustain crate** rather than
re-deriving the format from scratch. The existing project's Rhythmbox
read path is throwaway in the Sustain context — Sustain owns its own
library — but the Pioneer-write path is the load-bearing piece worth
porting.

What the reference project already produces on the target drive:

- `PIONEER/rekordbox/export.pdb` — the track database (tracks, artists,
  albums, genres, playlists), with the binary layout reverse-engineered
  from Rekordbox
- `PIONEER/USBANLZ/P{XXX}/{HASH}/ANLZ0000.DAT` — preview waveforms
- `PIONEER/USBANLZ/P{XXX}/{HASH}/ANLZ0000.EXT` — detailed waveforms and
  cue points, including the `PWAV` / `PWV2`–`PWV5` waveform encodings
- `Contents/{Artist}/{Album}/{Track}` — the audio files themselves, laid
  out in the hierarchy the on-drive database expects
- Pioneer's proprietary path-hash algorithm used to address the per-track
  analysis directory (`P{XXX}/{HASH}`), reverse-engineered from Rekordbox
  binaries
- BPM and musical-key detection (the reference project reports roughly
  87% and 72% accuracy respectively), with optional caching of the
  analysis results back into FLAC tags
- Artwork extraction with deduplication across tracks that share a cover

Constraints carried over from the reference project that Sustain should
address rather than inherit silently:

- the reference project re-exports the full library on every run; it
  does not do incremental sync. Sustain's sync panel needs incremental
  diffing (add/remove/update vs. what's already on the drive) since
  that's the whole point of the iPod-style workflow described above.
- the reference project cannot write back to MP3 tags due to an
  upstream library limitation. Sustain's tag-writing path (which has to
  exist anyway for ratings) is the right place to fix this; the export
  should reuse Sustain's tag writer instead of carrying the limitation
  forward.
- audio-analysis (BPM/key) is shared concern with the
  `## Audio analysis` item above and with `## Smart Shuffle`. The
  analysis pipeline should live in a single Sustain crate and be
  consumed by both the in-app columns/shuffle and the Pioneer export,
  not duplicated.

Concrete shape of the port (which crate, how the format code is split
from the Rhythmbox-specific code in the reference repo, licensing
review since both projects are the maintainer's own work, what to do
with the Python components that make up ~38% of the reference repo) is
to be decided at implementation time.

### CD encoding

Rip an audio CD into the library. Concrete shape (disc access library,
metadata lookup source, default encoding format, placement in the library)
is to be decided at implementation time.

### Library format conversion

Replace tracks in the library with a re-encoded copy in a different format —
mainly to replace bulky WAV files with something lighter (e.g. FLAC, or
MP3 320 kbps). The source files **are deleted** as part of the operation;
this is a replace, not a duplicate. Concrete shape (selection UI, target
format picker, batching, metadata preservation across the re-encode) is to
be decided at implementation time.

### Sort tags

Audio tag formats carry a parallel set of sort fields alongside the
display fields (ID3v2 `TSOP` / `TSOA` / `TSOT` / `TSO2` / `TSOC`; Vorbis
`ARTISTSORT` / `ALBUMSORT` / `TITLESORT` / `ALBUMARTISTSORT` /
`COMPOSERSORT`; the MP4 equivalents). They exist so that, for example,
"The Beatles" displays as written but sorts under **B**, "Björk" sorts as
"Bjork", and classical composers can be sorted "Last, First" while
displaying "First Last". Well-tagged libraries (MusicBrainz Picard, beets,
iTunes-sourced files) typically already carry them.

Sustain should read these fields on scan, store them alongside the
display fields, and prefer them in `ORDER BY` when present (falling back
to the display field otherwise).

Behaviour is controlled by a single preferences tickbox:

- **enabled** (default for well-tagged libraries): sort fields drive
  table ordering; display fields are shown as-is
- **disabled**: ordering uses the display fields only, useful for users
  who tag inconsistently or who want strict alphabetic-as-shown sorting

Eventual editing of sort fields belongs in the File Info editor and is
deferred to whenever that editor lands.

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

## Single-Instance Enforcement (Library Integrity)

Two Sustain instances pointed at the same on-disk library must not be allowed
to run concurrently. The threat is library-integrity corruption:

- SQLite writes from two processes interleave at the statement level, but
  Sustain's invariants (play counts, ratings, playlist membership, scan
  bookkeeping) span multiple statements and assume a single writer's
  view. A second instance can land a write between the read and write of
  the first instance's transaction and silently clobber state.
- Rating persistence writes to audio file metadata tags in addition to
  SQLite (per `CLAUDE.md`). Two instances rating the same track race on
  the file tag write and one update is lost — but the SQLite cache may
  reflect either, depending on commit order, so the file tag and the
  cached rating can diverge with no easy way to detect the drift.
- The filesystem watcher and the import pipeline both add tracks based
  on what they observe on disk. Two instances scanning the same root
  produce duplicate insertion attempts and conflicting de-duplication
  decisions.
- MPRIS/D-Bus registers a single bus name per instance. A second instance
  either steals the name or fails to claim it; in either case media keys
  and external clients (GNOME Shell, playerctl) target the wrong process
  unpredictably.

The check must be keyed to the **resolved data paths** (database file +
config file), not to the application id. A developer running the installed
package against the real library and a dev build against an isolated
`--local-scope` sandbox is legitimate and must keep working. Two processes
that resolve to the same database path is the case to refuse.

Mechanism (to be designed):

- on startup, acquire an exclusive advisory lock on the resolved SQLite
  database file (e.g. `flock(LOCK_EX | LOCK_NB)` on the file itself or a
  sidecar `.lock` file in the same directory). The lock is held for the
  process lifetime and released automatically on exit or crash.
- if the lock is already held, do not start a second main loop. Instead,
  signal the existing instance to raise/focus its window (the standard
  GTK pattern is `gtk::Application` with `G_APPLICATION_HANDLES_OPEN`
  and a unique application id; the second invocation forwards its
  command-line arguments to the running instance and exits).
- the unique application id used for the GTK side must be derived from
  (or include a hash of) the resolved database path, so the dev/prod
  coexistence case above resolves to two distinct GTK applications and
  neither single-instance check fires across them.
- if locking fails for a reason other than "already held" (permissions,
  read-only filesystem), surface a clear error and exit non-zero rather
  than continuing without a lock.

Non-goals for the first pass:

- no cross-machine locking (NFS/SMB-mounted libraries are out of scope;
  advisory file locks over network filesystems are unreliable and the
  product target is local libraries on local disks)
- no detection of an externally-modified database (a different SQLite
  client editing the file behind Sustain's back is the user's problem)
- no UI for forcibly stealing the lock from a stale instance; if a crash
  leaves a stale lock, document that the file lock is OS-released on
  process exit and that a stale sidecar `.lock` file (if that approach
  is chosen) can be deleted manually

## Distribution

Target Debian as the primary distribution platform. The project should produce a
`.deb` package that installs cleanly on Debian stable and Ubuntu LTS without
requiring users to build from source or add third-party repositories.

Debian is not a generic deployment target — it is the maintainer's daily-driver
distribution (Debian testing) and Sustain is intended to be their everyday
music player. Two consequences flow from that:

1. **An easy local-package path is required from day one.** The maintainer
   must be able to produce an installable `.deb` of the current working
   tree on their own machine without friction. The existing
   `[package.metadata.deb]` block in `crates/app/Cargo.toml` is the
   starting point.
2. **Distribution through Debian's official channels is an explicit goal,
   not an afterthought.** The packaging must be shaped so it can plausibly
   reach the official Debian archive. Concrete process and policy
   compliance details are to be worked out when we get there.

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

## Pre-Release Performance Pass

Once the feature set is frozen for the first public release, do a dedicated
performance optimization pass before shipping. Treat this as its own phase,
not as opportunistic cleanup folded into feature work — interleaving
optimization with feature development tends to produce premature
micro-optimizations in the wrong hot paths and leaves the real bottlenecks
(typically discovered only once the system is whole) unaddressed.

Scope:

- profile under a realistic large-library workload (tens of thousands of
  tracks, deep playlist hierarchies, full artwork cache) on the
  Debian-first target hardware, not on a synthetic empty database
- measure cold-start time, library scan throughput, search latency at
  each keystroke, table scroll smoothness with all columns visible,
  Albums-view tile rebuild on selection, and SQLite query time on the
  hot read paths
- treat the existing open performance items (`## Library Scan
  Performance`, `## Gapless Track-to-Track Playback`, and the
  `## Pre-Release Punch List` entries that touch hot paths) as the
  starting checklist, not the full surface — those captured what was
  visible at the time, not necessarily what will dominate once the rest
  of the product is in place
- only after measurement, optimize: SQL index review, query
  consolidation, model rebuild diffing, render avoidance, allocation
  reduction in cell factories, artwork cache shape, async/background
  task scheduling
- audit what can be parallelized across cores. Modern desktop CPUs ship
  with 8–16+ cores and Sustain currently leaves nearly all of them idle.
  Library scans, metadata reads, artwork decoding, palette extraction,
  and search index rebuilds are all embarrassingly parallel over
  independent tracks/files and should be candidates for a worker pool
  (e.g. `rayon` for CPU-bound passes, dedicated background threads for
  I/O-bound ones). The GTK main thread stays single-threaded by design,
  but everything feeding it does not have to. Measure first, then
  parallelize the passes whose serial time actually dominates.

Non-goals for this pass:

- no new features, no UX changes that are not directly required by a
  measured performance improvement
- no speculative optimization without a profile showing the cost first
- no rewrites of subsystems that profile well even if their code reads
  awkwardly — defer those to a separate refactor pass

The pass ends with a documented before/after for each metric and a
short note explaining any deliberately-deferred bottleneck (with the
reason it was left alone). Performance regressions found after this
pass should be treated as bugs against the established baselines.

## Pre-Release Data-Loss Audit

A dedicated audit pass for **data-loss prevention** is a hard gate before
any public release. Music libraries are precious — they represent years of
ripping, tagging, rating, and listening — and a single bad code path that
truncates a file, overwrites a tag with empty data, or clears a play count
on rescan is not recoverable from the user's perspective. This audit is
separate from, and complementary to, the security audit: security is
"can an attacker hurt the user"; this is "can our own code hurt the user."

**Scope.** Every code path that writes to, mutates, replaces, or deletes:

- audio files on disk (atomic-replace pipelines, tag writers, artwork
  embed/strip, any future format conversion)
- the SQLite library database (ratings, play counts, skip counts,
  last-played / last-skipped timestamps, playlist membership, smart
  playlist definitions, folder hierarchy, custom metadata)
- on-disk caches that, if corrupted, could mask or destroy the
  authoritative state (artwork cache, search index, lyric cache)

**Audit checklist.** Each candidate path is reviewed for:

- **Atomicity.** Tag writes must be write-to-temp + rename-into-place, not
  in-place truncation. A power loss or kill -9 mid-write must leave the
  original file intact. Same for SQLite — wrap multi-statement edits in a
  transaction.
- **Rescan idempotence.** Rescanning a library MUST NOT clobber values that
  live only in SQLite (e.g. skip count in Vorbis Comments). The general
  tag-mirroring rule says file tags win on conflict — but only for fields
  that *have* a tag. SQLite-only fields are authoritative for their
  formats and must survive a full rescan.
- **Empty-string vs. missing.** Distinguish "user cleared the field" from
  "we failed to read it." A read failure must never propagate as
  "title = ''" and overwrite a valid file tag on the next save. Read
  failures are explicit errors, not empty values.
- **Diff direction.** Metadata commits use a diff-against-baseline model.
  Every diff must be verified to write *only* the fields the user
  actually changed. A buggy diff that emits every field on every save
  amplifies the blast radius of any single bug into all fields.
- **Destructive commands gated.** Delete-file, remove-from-library, clear
  ratings, reset play counts, and similar irreversible actions require
  explicit user confirmation and must never be the default of a keyboard
  shortcut or auto-cleanup pass. No code path may reach a "rm" or a tag
  wipe without a confirmed user intent in the call stack.
- **Background tasks vs. live edits.** A long-running scan, organize,
  metadata-write, or import job must cooperate with concurrent user
  edits — last-writer-wins is acceptable only if both writers have the
  same baseline. If the user edits a track while a background job is
  rewriting that track's tags, the audit verifies which write wins and
  whether the loser surfaces an error.
- **Filesystem moves and renames.** "Organize library" / "consolidate"
  flows must copy-then-verify-then-delete-source, not move-and-pray. A
  failed move must leave the source file intact and a clear error trail.
- **Removable media and missing files.** A track on an unmounted drive
  must register as "unavailable", not be silently deleted from the
  library on the next scan. The library row is the user's record that
  the track existed; deleting it because the file is currently
  unreachable is data loss.
- **Database vacuum / migration paths.** Once migrations land
  post-release, every migration is reviewed for "is the source data
  recoverable if this migration fails halfway?" Pre-release we don't
  carry migrations, but the audit covers the schema-rebuild path used
  during development too.
- **Backup-friendliness.** The library DB and any out-of-tree data
  (artwork cache, etc.) should be in known, single-rooted locations so
  the user can back them up with one rsync. The audit verifies nothing
  important lives in a path the user wouldn't think to copy.

**Method.** Same shape as the security audit — independent passes by LLM
coding agents (Codex and Claude) over the codebase with this checklist,
plus a manual review of any path either agent flagged. Findings classified
as: must-fix-before-release / fix-but-acceptable-with-mitigation /
documented-known-limitation. Anything that can silently destroy user data
is must-fix; failures that surface a clear error to the user can be
weighed against effort.

**Non-goals.** Not a performance pass, not a UX pass, not a
feature-completeness review. The single question is: "can this code lose
a user's music or library state, ever, under any code path?"

## Pre-Release Security Audit

A security audit of the codebase by LLM coding agents (Codex and Claude,
run independently) is a hard gate before any public release.
