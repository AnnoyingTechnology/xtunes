# Sustain Feature Handoff

This is the next-agent handoff after the refactor pass. Treat it as an
implementation map, not as a product spec replacement. `REFACTOR.md` remains
the detailed refactor rationale.

## Current Guarantees

- Runtime command dispatch is explicit. Adding an `ApplicationCommand` must
  force deliberate app-runtime handling.
- Mutable UI actions should go through `UiCommandController`; GTK widgets should
  not mutate durable model state directly.
- Ratings and metadata writes go to audio tags first, then SQLite/runtime cache.
- Regular playlist commands are implemented and tested in runtime/store.
- The pre-release database schema is disposable. Edit authoritative `CREATE
  TABLE` statements directly; do not add migrations or compatibility shims.
- Last known verification before this handoff: `cargo test`, `cargo fmt
  --check`, and `cargo check` passed after the refactor.

## Remaining Work Order

### 1. Smart Playlists

Domain scaffolding now exists in `sustain_domain::smart_playlist`: saved smart
playlists have an id, name, rule set, match kind, optional positive limit, and
typed rule fields/operators. The next work is persistence and evaluation.

Visual/product reference:

- `screenshots/Clever Playlists/`

Before implementing the GTK smart-playlist editor, inspect those screenshots
and draft a short "ported vs not ported" decision list for maintainer approval.
Do not assume every iTunes control should be copied exactly; Sustain should keep
the dense, predictable workflow while staying native GTK and maintainable.

Implement in this order:

1. Add store tables for `smart_playlists` and ordered `smart_playlist_rules` by
   editing the current schema definition directly.
2. Add `LibraryStore` methods to save, load, delete, and list smart playlists.
3. Add rule serialization/deserialization tests before runtime/UI wiring.
4. Add rule evaluation/query translation in store/search/runtime code, not GTK.
5. Add `ApplicationCommand` variants only when runtime can handle them
   deliberately.
6. Add UI last: sidebar rows, editor surface, and context-menu actions.

Acceptance tests to add:

- save/load preserves all rule variants, order, match kind, and limit
- invalid ids/names/rules are rejected at domain/runtime boundaries
- all-match vs any-match behavior is tested with mixed matching tracks
- relative date rules use an injectable clock, not `SystemTime::now()` directly
- smart playlist deletion removes rules and sidebar state

### 2. Playlist UI Completion

Regular playlist behavior exists below the UI. The sidebar and context menus
still need to become real playlist surfaces.

Implement:

- sidebar rows backed by `ApplicationRuntime::playlists()`
- selecting a playlist changes the track query to `LibraryQuery::in_playlist`
- context-menu add/remove actions use `TrackContextAction` descriptors
- rename/delete/create route through `UiCommandController`
- playlist track reordering stays a runtime command, not a GTK-only row move
- drag tracks from table/search/album surfaces to playlist sidebar rows by
  dispatching `AddTracksToPlaylist`
- drag rows within a regular playlist by dispatching `MovePlaylistEntry`

Test through runtime/store first; use GTK tests only for action availability and
dispatch/drop-target wiring. Drag/drop from the file manager is a separate
import feature, not the same path as internal track-to-playlist drops.

### 3. Library Organization Mode

Settings will need a "Keep my library organized" checkbox. This is an
alternative library-management mode, not just a preference field: tracks added
while the mode is enabled are managed by Sustain and moved under the library root
using a clean metadata-derived path such as artist/album/track.ext.

Implement this as a foundation before surfacing the checkbox:

- add an explicit library management setting under `UserSettings.library`
- define a domain-level path planner for managed tracks
- decide the exact path format with maintainer validation before coding it:
  artist fallback, album fallback, track/disc numbering, title vs filename,
  extension preservation, Unicode policy, forbidden characters, and max length
- resolve filename collisions deterministically before moving files
- perform filesystem movement before updating SQLite/runtime state
- keep failures visible and leave the library model pointing at the original
  valid file when a move fails
- do not reorganize the existing library automatically when the checkbox is
  toggled; that needs an explicit future "consolidate/reorganize library"
  command
- make imported/added files go through one application command path shared by
  file chooser, file-manager drops, and future importer flows

Acceptance tests to add:

- path planning handles missing artist/album/title metadata
- path planning preserves file extensions and rejects unsafe relative paths
- collision resolution is stable and covered by tests
- successful managed add moves the file, stores the new relative path, and keeps
  metadata/rating behavior intact
- failed movement leaves the store/runtime unchanged and reports an error
- toggling the setting alone never moves existing files

Do not add the checkbox until the managed-add command path exists behind it.

### 4. File Info

`ApplicationCommand::UpdateMetadata` is already durable. Build File Info as a
GTK editing surface over that command path.

Visual/product reference:

- `screenshots/Track Info/`

Before implementing the Track Info window, inspect those screenshots and draft
a short tab-by-tab porting decision for maintainer approval. The decision should
separate first-pass metadata fields from later artwork, lyrics, sorting,
options, and statistics work.

Implement:

- a small File Info view model built from selected `Track` snapshots
- single-track fields first; multi-track mixed-value editing only after the
  field-change model is explicit for it
- save dispatches `UpdateMetadata`; rows refresh from runtime state
- "File Info" is a `TrackContextAction`, with deterministic selection rules
- artwork and lyrics edit commands only after metadata/domain/store support
  exists for those payloads

Do not let File Info directly mutate `TrackTableRow`, SQLite, or tags.

### 5. Shared Artwork And Palette Cache

Album view and now-playing still decode/read artwork independently. Add one
shared cache before implementing album palettes, artwork zoom, or richer
now-playing artwork behavior.

Palette requirement:

- extract two artwork-derived colors, not just one dominant color
- the second artwork color must visibly contrast with the first because album
  detail surfaces need secondary artwork accents
- keep readable text/icon foreground as a separate derived value; do not treat
  black/white foreground as the secondary artwork color
- the current `ArtworkPalette` helper only models dominant background plus
  readable foreground, so upgrade that contract before adding secondary album
  accents

Recommended shape:

- UI data-layer service, unless runtime needs palette data for non-UI behavior
- cache key: stable track id plus relative path, invalidated when metadata/path
  changes
- value: decoded GTK texture plus palette summary containing primary artwork
  color, contrasting secondary artwork color, and readable foreground
- metadata/file reads happen off the hot UI path where possible
- albums and now-playing consume the same cache API

Acceptance:

- missing artwork is cached as a known absence
- two-color extraction is tested with artwork samples that contain multiple
  saturated regions and with near-monochrome artwork that needs a safe fallback
- corrupted artwork reports a command/status-visible failure only when user
  initiated, otherwise degrades quietly
- light/dark palette styling remains readable

### 6. Follow-Up Splits

Only split these when the touched feature needs it:

- `now_playing.rs`: marquee rendering, seek/progress handling, artwork view, and
  option buttons can become separate modules.
- `albums.rs`: split grouping/view model, grid/tile rendering, expanded detail,
  palette styling, and context-menu integration.
- preferences: add section widgets/view models as new settings arrive.

## Guardrails

- No hacks, legacy branches, schema migrations, or compatibility paths during
  pre-release.
- No UI-only success for commands that can fail durably.
- No direct GTK mutation for ratings, metadata, playlists, smart playlists, or
  library membership.
- No new abstraction unless it removes real coupling or matches an established
  local pattern.
- Keep tests close to behavior: domain rules in `sustain-domain`, persistence in
  `sustain-library`, runtime command behavior in `sustain-app-runtime`, GTK only
  for UI wiring/availability.
