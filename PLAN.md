# xTunes Architecture Plan

## Summary

`xTunes` is a Linux-only, Debian-first music library/player built with Rust,
GTK4, GStreamer, and SQLite. The product target is an iTunes 11-like desktop
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
- playlist entries
- metadata
- ratings
- play statistics
- settings
- schema migrations

## Application Flow

Use command/query separation.

Commands include:

- play selected track
- pause/resume/stop
- seek
- set rating
- create, rename, and delete playlist
- add/remove/reorder playlist entries
- update metadata
- update settings

Queries include:

- list tracks with filter and sort
- list playlists
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
- settings dialog shell, especially library path and manual scan
- native light and dark theme behavior
- keyboard behavior, especially spacebar play/pause
- empty and populated library states

Album view can remain rough during this phase. Advanced browsing views,
visualizers, streaming, device sync, cloud sync, folder sync, and multi-machine
sync are out of scope.

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
