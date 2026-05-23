# Sustain Refactor Plan

This is a prioritized refactor list for the next feature wave: real shuffle and
repeat behavior, richer preferences, larger track context menus, regular
playlists, smart playlists, a tabbed File Info panel/window for richer metadata
editing, managed library organization, and the remaining items in `PLAN.md`.

The current codebase does not need a rewrite. The core crate boundaries are
still sound: domain, runtime, store, metadata, playback, settings, and GTK UI are
separate enough to keep building. The risky parts are places where growth would
turn missing behavior into silent no-ops, or where GTK widgets currently mutate
local state instead of sending durable application commands.

The planned File Info surface matters for this refactor because it will edit
many durable track properties, including ID3-style fields, artwork, lyrics, and
ratings. It should not become a GTK-only dialog that mutates row state. It needs
the same command, metadata, store, notification, context-action, and CSS
foundations as ratings, playlists, and other track actions.

## Priority 0 - Safety Gates

These should be done before adding more behavior on top of the current shell.

### 1. Make runtime command handling exhaustive - Done

`ApplicationRuntime::handle_command` no longer has a catch-all arm that silently
accepts unimplemented commands. Unsupported command variants now return
`ApplicationRuntimeError::UnsupportedCommand`, and tests cover intentional
runtime behavior for every current command variant.

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

### 2. Introduce a playback queue/order model - Core Done

Shuffle and repeat are no longer only loose runtime booleans. Runtime owns a
domain-level playback queue with ordered track ids, current track, shuffle order,
and explicit repeat mode. Current playback still starts from the library source;
album, playlist, and search-result playback should be added by feeding their
track ids into the same queue machinery.

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
  app-runtime, not in `sustain-playback`.

Acceptance criteria:

- Next/previous behavior is tested for normal order, shuffle, repeat-off,
  repeat-one, repeat-all, missing tracks, and end-of-queue.
- Album and playlist playback do not special-case shuffle in GTK.

### 3. Route rating changes through runtime and metadata - Runtime Path Done

The table rating cell no longer mutates only its local `TrackTableRow`. Rating
changes dispatch `ApplicationCommand::SetRating`; runtime writes the file tag
first through `MetadataService`, then updates the SQLite cache and runtime track
state. The table does not optimistically update on command failure; a
user-visible command failure notification still belongs with the GTK
command/notification controller in Priority 1.

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

### 4. Split app-runtime by responsibility - Production Code Done

`crates/app_runtime/src/lib.rs` now keeps `ApplicationRuntime` as the public
facade and shared runtime types. Command dispatch, playback behavior, library
mutation, and library scanning/reconciliation live in focused modules. The
remaining centralized tests are mostly facade-level tests that exercise behavior
across those modules.

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

### 5. Add a UI command/notification controller - Core Done

GTK now has a `UiCommandController` that owns the shared runtime handle,
dispatches commands, and routes command failures into the status bar. Context
menus, rating cells, preferences, keyboard shortcuts, titlebar playback controls,
now-playing controls, and album playback all use this command path. Views still
borrow runtime state for read-only snapshots and artwork reads; the remaining
narrowing can happen as the affected modules are split.

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

### 6. Split the main GTK shell module - Done

`crates/ui_gtk/src/lib.rs` builds the window, titlebar, mode bar, sidebar,
content stack, callbacks, keyboard shortcuts, status refresh, CSS, and window
chrome. It is already too broad for the planned UI work.

Window chrome, titlebar construction/playback controls, mode switching, the
playlist sidebar shell/content paned, content-stack construction, app CSS, and
main-window orchestration now live in focused modules. `lib.rs` is back to being
the public entrypoint and crate-level wiring.

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

### 7. Split and harden the track table - Done

`track_table.rs` owns row view models, file-type formatting, column definitions,
column visibility actions, cell factories, selection status, playing-state
status, rating widgets, and context-menu installation. The table is the core
product surface, so this should be easier to evolve.

The table is now split into row model/formatting, column contract, and GTK cell
machinery modules. Rating cells dispatch through command callbacks and no longer
perform hidden durable-state mutation.

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

### 8. Build a playlist and smart-playlist foundation - Regular Playlists Done

Regular playlist commands now run through app-runtime and `LibraryStore`, with
tests for create, rename, delete, add, remove, move, and playlist query
ordering. Smart playlist domain scaffolding now exists; store persistence, rule
evaluation, runtime commands, and UI remain future work.

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
- Dragging tracks to playlist sidebar rows dispatches playlist commands instead
  of mutating GTK-only row state.
- Smart playlist rules can be saved, loaded, and evaluated without UI-specific
  logic.

### 9. Turn the context menu into an action model - Done

The current direct GTK popover is acceptable for remove/trash. It will not scale
well to actions like add to playlist, open File Info, edit metadata, show in
folder, consolidate, locate missing file, or smart-playlist operations unless
availability and dispatch are modeled explicitly.

Track context menus are now built from `TrackContextAction` descriptors with
stable ids, labels, destructive markers, selection requirements, callbacks, and
confirmation policy.

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

### 10. Prepare the File Info metadata-editing foundation - Runtime Foundation Done

The future File Info panel/window will have multiple tabs and custom styling for
editable metadata, artwork, lyrics, and related track details. It should sit on
top of the application model instead of becoming a self-contained GTK editor.

`ApplicationCommand::UpdateMetadata` now writes through `MetadataService` first
and then updates SQLite/runtime cache state. `TrackMetadata` owns field-change
application logic. Artwork and lyrics edit types remain future work and should
be added deliberately when the UI needs them.

Scope:

- Extend domain metadata-edit types deliberately before adding UI fields,
  including artwork and lyrics support when those edits are actually needed.
- Implement metadata update runtime commands so File Info saves through
  `MetadataService` first and then updates SQLite/runtime cache state.
- Keep the File Info window as a GTK surface backed by a small view model and
  command dispatch, not by direct mutation of table rows or copied track state.
- Reuse the context-action model so "File Info" has deterministic availability
  for single selection, multi-selection, playlist rows, album rows, and missing
  tracks.
- Keep File Info CSS in the central app CSS layer with explicit native
  light/dark behavior.

Acceptance criteria:

- File Info can be opened from track actions without adding bespoke popover
  routing.
- Metadata saves have one durable command path shared with inline table edits.
- Artwork and lyrics edits have domain/runtime/store support before GTK tabs are
  wired.

### 11. Reshape settings before expanding preferences - Core Done

`UserSettings` and the preferences window currently revolve around a single
library path. That is fine today but should not be copied for each new setting.

Settings are now grouped under `UserSettings.library`, TOML persists a
`[library]` section, and callers use a `library_path()` accessor instead of
reaching through ad hoc top-level fields.

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

### 12. Prepare library organization mode

Settings will need a "Keep my library organized" checkbox. This is not just UI
state: it means newly added tracks become managed by Sustain and are moved into a
clean metadata-derived path under the library root, such as
artist/album/track.ext.

Scope:

- Add an explicit library management setting under grouped library settings.
- Introduce a tested path planner for managed files before adding the checkbox.
- Decide the exact path contract with maintainer validation: fallbacks for
  missing artist/album/title, track/disc numbering, extension preservation,
  Unicode/forbidden-character handling, max filename length, and collision
  resolution.
- Move files through one application command path shared by file chooser,
  file-manager drops, and future import flows.
- Perform filesystem movement before updating SQLite/runtime cache state.
- Do not reorganize the existing library just because the setting is toggled;
  that belongs to a later explicit consolidate/reorganize command.

Acceptance criteria:

- Managed-add tests cover path planning, collision resolution, successful moves,
  failed moves, and missing metadata fallbacks.
- A failed move leaves the store/runtime pointing at the original valid file.
- The preferences checkbox is not surfaced until the behavior behind it exists.

### 13. Extract and centralize app CSS - Done

Custom CSS is currently installed from a large string in `ui_gtk/src/lib.rs`.
Theme behavior is important for Sustain, and this will become harder to audit as
more views, including File Info, are added.

Static CSS now lives in `ui_gtk/src/app.css` and is installed through the
focused `app_css` module.

Scope:

- Move static CSS into an `app_css` module or GTK resource file.
- Keep theme-aware tokens centralized.
- Continue relying on native GTK light/dark behavior.
- Only force colors where Sustain controls both background and foreground.

Acceptance criteria:

- Theme CSS can be reviewed without reading window construction code.
- New custom surfaces have explicit light/dark behavior.

### 14. Share artwork and palette caching

Album view and now-playing both read artwork. `PLAN.md` calls for shared
dominant-color detection so album detail surfaces and now-playing artwork use
one cached source of truth.

Scope:

- Add an artwork cache/service at runtime or a dedicated UI data layer.
- Store decoded texture/palette results per track or path.
- Extract two artwork-derived colors: a primary artwork color and a contrasting
  secondary artwork color for album accents. Keep readable text/icon foreground
  as a separate derived value, not as the secondary artwork color.
- Keep metadata reading off the hot UI path where possible.

Acceptance criteria:

- Album view and now-playing do not duplicate artwork decoding or palette
  calculation.
- Non-square artwork gutter coloring and album detail palettes consume the same
  palette data, including secondary artwork accents where needed.

### 15. Split now-playing internals - Started

`now_playing.rs` contains playback controls, progress seeking, artwork loading,
marquee text, option buttons, and layout. It can support the current UI, but
shuffle/repeat and artwork/lyrics features will add pressure.

Pure now-playing model behavior, including title/subtitle formatting, time text,
playback position extraction, and progress fraction math, is split into a tested
submodule. Marquee drawing, seek gesture wiring, artwork rendering, and playback
option controls can be split further when those areas are next changed.

Scope:

- Split marquee label, progress/seek handling, playback option controls, and
  artwork rendering into focused modules.
- Keep command dispatch outside low-level drawing/layout helpers.

Acceptance criteria:

- Player behavior changes do not require editing marquee rendering code.
- Future artwork zoom/lyrics work has a clear home.

### 16. Split album view when album work resumes

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

1. Done: make `ApplicationRuntime::handle_command` exhaustive.
2. Core done: implement the playback queue/order model.
3. Runtime path done: implement durable rating commands through metadata and
   store updates.
4. Core done: add the GTK command/notification controller.
5. Production code done: split app-runtime by responsibility.
6. Done: split `ui_gtk/src/lib.rs` and extract app CSS.
7. Done: split and harden `track_table.rs`.
8. Done for regular playlists: runtime/query support is implemented.
9. Done: add the context-menu action model.
10. Runtime foundation done: metadata updates share the durable command path.
11. Remaining feature foundation: add smart playlist domain/store/query support.
12. Core done: reshape settings/preferences for multiple sections.
13. Remaining feature foundation: prepare managed library organization mode.
14. Done: extract CSS. Remaining: shared artwork/palette caching.
15. Started: split now-playing pure model behavior; split rendering/control
    submodules further as touched.

## Next-Agent Handoff

`FEATURE_HANDOFF.md` contains the concise implementation map for the remaining
feature skeletons: smart playlists, playlist UI completion, library
organization mode, File Info, shared artwork/palette caching, and the follow-up
GTK splits.
