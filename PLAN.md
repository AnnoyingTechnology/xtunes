# xTunes Architecture Plan

## Summary

`xTunes` is a Linux-only, Debian-first music library/player built with Rust,
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
where they already are. xTunes must not rename, move, copy, or reorganize files
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
not add an xTunes theme picker. Keep theme-aware CSS tokens centralized and
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
