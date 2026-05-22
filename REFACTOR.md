# xTunes Refactor Plan

This is a prioritized refactor list for the next feature wave: real shuffle and
repeat behavior, richer preferences, larger track context menus, regular
playlists, smart playlists, and the remaining items in `PLAN.md`.

The current codebase does not need a rewrite. The core crate boundaries are
still sound: domain, runtime, store, metadata, playback, settings, and GTK UI are
separate enough to keep building. The risky parts are places where growth would
turn missing behavior into silent no-ops, or where GTK widgets currently mutate
local state instead of sending durable application commands.

## Priority 0 - Safety Gates

These should be done before adding more behavior on top of the current shell.

### 1. Make runtime command handling exhaustive

`ApplicationRuntime::handle_command` currently has a catch-all arm that silently
accepts unimplemented commands. That is unsafe for upcoming work because
`SetRating`, playlist commands, and metadata commands can look wired from GTK
while doing nothing in the application model.

Scope:

- Remove the catch-all match arm from `ApplicationRuntime::handle_command`.
- Either implement each `ApplicationCommand` or return a specific
  `ApplicationRuntimeError::UnsupportedCommand` while the feature is genuinely
  unavailable.
- Add tests that prove every command variant has an intentional runtime result.
- Prefer domain/application errors over UI-only assumptions.

Acceptance criteria:

- Adding a new `ApplicationCommand` forces an app-runtime compile error until it
  is handled deliberately.
- No user action can appear to succeed while the runtime ignored it.

### 2. Introduce a playback queue/order model

Shuffle and repeat are currently stored as booleans. Next/previous still walk the
library in sequential order, and there is no explicit playback source for
library, album, playlist, or search-result playback. That is not a solid base
for player work.

Scope:

- Add a domain-level playback queue/order type that owns:
  - the ordered track ids for the active playback source
  - the current track id/index
  - shuffle order
  - repeat mode
- Replace the boolean repeat option with an explicit repeat mode if needed
  (`Off`, `One`, `All`) before behavior spreads through the UI.
- Make play-library, play-album, play-playlist, and play-selection feed the same
  queue machinery.
- Keep GStreamer as a playback service only; queue semantics belong in domain or
  app-runtime, not in `xtunes-playback`.

Acceptance criteria:

- Next/previous behavior is tested for normal order, shuffle, repeat-off,
  repeat-one, repeat-all, missing tracks, and end-of-queue.
- Album and playlist playback do not special-case shuffle in GTK.

### 3. Route rating changes through runtime and metadata

The table rating cell currently updates only its local `TrackTableRow`. That
does not persist to SQLite or audio file metadata, and it bypasses the product
requirement that file tags are the durable rating source.

Scope:

- Implement `ApplicationCommand::SetRating` in the runtime.
- Resolve the track's absolute path from settings and track location.
- Call `MetadataService::write_rating` first, then update the cached track and
  `LibraryStore`.
- Remove direct rating mutation from GTK row objects.
- Make the table refresh from runtime state after the command completes.

Acceptance criteria:

- A rating click updates the file tag, SQLite cache, runtime track, and visible
  table row through one command path.
- Failures are surfaced instead of being hidden by the cell factory.

## Priority 1 - Feature Growth Refactors

These are the next layer after the safety gates. They prevent the GTK shell and
runtime from becoming hard to change as features are added.

### 4. Split app-runtime by responsibility

`crates/app_runtime/src/lib.rs` currently contains settings, library scanning,
library mutation, playback command handling, track availability, scan
reconciliation, task state, and tests in one large module.

Scope:

- Keep `ApplicationRuntime` as the public facade.
- Move focused behavior into modules such as:
  - `commands`
  - `playback`
  - `library_mutation`
  - `library_scan`
  - `settings`
  - `playlists`
- Keep tests near the behavior they verify.

Acceptance criteria:

- Runtime feature work no longer requires editing one central thousand-line
  module for every concern.
- Public runtime API remains small and intentional.

### 5. Add a UI command/notification controller

GTK currently passes `Rc<RefCell<ApplicationRuntime>>` into many widgets and
uses ad hoc callbacks for refresh. Some command failures are intentionally
ignored, and batch operations report success if any track succeeded.

Scope:

- Introduce a small GTK-side controller or handle that owns runtime access,
  dispatches commands, and emits typed UI notifications.
- Centralize command error handling and status-bar messages.
- Let views receive narrow callbacks or state snapshots instead of borrowing the
  runtime directly wherever possible.
- Preserve GTK main-thread ownership.

Acceptance criteria:

- Context menus, rating cells, preferences, keyboard shortcuts, and playback
  controls all dispatch through one consistent path.
- Batch track operations have clear partial-failure behavior.

### 6. Split the main GTK shell module

`crates/ui_gtk/src/lib.rs` builds the window, titlebar, mode bar, sidebar,
content stack, callbacks, keyboard shortcuts, status refresh, CSS, and window
chrome. It is already too broad for the planned UI work.

Scope:

- Move focused pieces into modules such as:
  - `main_window`
  - `titlebar`
  - `mode_bar`
  - `sidebar`
  - `status_bar`
  - `window_chrome`
  - `app_css`
- Keep `lib.rs` mostly as public entrypoint and crate-level wiring.

Acceptance criteria:

- Adding preferences sections or context-menu behavior does not require touching
  unrelated window construction code.
- CSS installation is no longer embedded inside the application wiring module.

### 7. Split and harden the track table

`track_table.rs` owns row view models, file-type formatting, column definitions,
column visibility actions, cell factories, selection status, playing-state
status, rating widgets, and context-menu installation. The table is the core
product surface, so this should be easier to evolve.

Scope:

- Split into focused modules:
  - row model and formatting
  - column definitions
  - cell factories
  - rating cells
  - status/playing indicator
  - selection and context-menu integration
- Replace cell-local mutations with command callbacks for mutable data.
- Give table actions explicit availability rules as context-menu features grow.

Acceptance criteria:

- Rating, sorting, column visibility, and context-menu work can be changed
  independently.
- The table remains dense and native-GTK, with no hidden app-state mutations.

### 8. Build a playlist and smart-playlist foundation

The store already has basic regular playlist persistence, but runtime commands
for playlists are ignored. Smart playlists do not have domain/store support yet.

Scope:

- Implement regular playlist runtime commands against `LibraryStore`.
- Add query support for `LibraryQuery::in_playlist`.
- Define smart playlist domain types as saved library queries/rules before
  adding UI.
- Update the pre-release SQLite schema directly for smart playlists and rules;
  do not add migrations or legacy compatibility code.
- Keep smart playlist evaluation in domain/app-runtime/search logic, not GTK.

Acceptance criteria:

- Playlist sidebar and context-menu "add to playlist" actions are backed by real
  runtime commands and tests.
- Smart playlist rules can be saved, loaded, and evaluated without UI-specific
  logic.

### 9. Turn the context menu into an action model

The current direct GTK popover is acceptable for remove/trash. It will not scale
well to actions like add to playlist, edit metadata, show in folder, consolidate,
locate missing file, or smart-playlist operations unless availability and
dispatch are modeled explicitly.

Scope:

- Define `TrackContextAction` descriptors with id, label, destructive flag,
  selection requirements, and command builder.
- Build the popover from those descriptors.
- Keep destructive confirmation as a reusable app-owned dialog helper.
- Avoid returning to fragile `gio::Menu` action routing unless action targets
  are modeled and tested.

Acceptance criteria:

- New context-menu items are added by declaring an action, not by hand-wiring
  more bespoke popover button code.
- Availability is deterministic for single selection, multi-selection,
  playlist rows, album rows, and missing tracks.

### 10. Reshape settings before expanding preferences

`UserSettings` and the preferences window currently revolve around a single
library path. That is fine today but should not be copied for each new setting.

Scope:

- Group settings by meaning in domain structs when more settings arrive.
- Keep TOML persistence explicit and simple.
- Move preferences UI into section widgets backed by a settings view model.
- Decide which settings apply immediately and which require explicit save/apply.
- Keep manual scan as an application command/status concern, not preferences
  window state.

Acceptance criteria:

- Adding a new setting does not require scattering ad hoc entry/button logic
  through one modal constructor.
- Settings save/load tests cover the full settings document.

## Priority 2 - Containment And Cleanup

These are valuable, but they should follow the command, playback, rating,
playlist, and UI-controller work unless a touched feature naturally requires
them.

### 11. Extract and centralize app CSS

Custom CSS is currently installed from a large string in `ui_gtk/src/lib.rs`.
Theme behavior is important for xTunes, and this will become harder to audit as
more views are added.

Scope:

- Move static CSS into an `app_css` module or GTK resource file.
- Keep theme-aware tokens centralized.
- Continue relying on native GTK light/dark behavior.
- Only force colors where xTunes controls both background and foreground.

Acceptance criteria:

- Theme CSS can be reviewed without reading window construction code.
- New custom surfaces have explicit light/dark behavior.

### 12. Share artwork and palette caching

Album view and now-playing both read artwork. `PLAN.md` calls for shared
dominant-color detection so album detail surfaces and now-playing artwork use
one cached source of truth.

Scope:

- Add an artwork cache/service at runtime or a dedicated UI data layer.
- Store decoded texture/palette results per track or path.
- Keep metadata reading off the hot UI path where possible.

Acceptance criteria:

- Album view and now-playing do not duplicate artwork decoding or palette
  calculation.
- Non-square artwork gutter coloring and album detail palettes consume the same
  palette data.

### 13. Split now-playing internals

`now_playing.rs` contains playback controls, progress seeking, artwork loading,
marquee text, option buttons, and layout. It can support the current UI, but
shuffle/repeat and artwork/lyrics features will add pressure.

Scope:

- Split marquee label, progress/seek handling, playback option controls, and
  artwork rendering into focused modules.
- Keep command dispatch outside low-level drawing/layout helpers.

Acceptance criteria:

- Player behavior changes do not require editing marquee rendering code.
- Future artwork zoom/lyrics work has a clear home.

### 14. Split album view when album work resumes

`albums.rs` is sizable, but album view is explicitly secondary to the Songs
table right now. It should be split when album features are next touched.

Scope:

- Separate album grouping/view models, grid layout, tile widgets, expanded
  detail rows, palette styling, artwork cache use, and context-menu integration.

Acceptance criteria:

- Album playback and context-menu behavior go through the same runtime/action
  paths as Songs.
- Palette styling remains readable in native light and dark modes.

## Not Required Before Continuing

- Do not rewrite the crate boundaries. They match the project direction.
- Do not add migration code for schema changes during pre-release development.
- Do not replace GTK, GStreamer, SQLite, or lofty.
- Do not abstract every widget preemptively. Refactor the table, runtime, and
  command paths first because they are the features' load-bearing pieces.

## Recommended Execution Order

1. Make `ApplicationRuntime::handle_command` exhaustive.
2. Implement the playback queue/order model.
3. Implement durable rating commands through metadata and store updates.
4. Add the GTK command/notification controller.
5. Split app-runtime by responsibility.
6. Split `ui_gtk/src/lib.rs`.
7. Split and harden `track_table.rs`.
8. Implement regular playlist runtime/query support.
9. Add the context-menu action model.
10. Add smart playlist domain/store/query support.
11. Reshape settings/preferences for multiple sections.
12. Extract CSS and shared artwork/palette handling as touched.

