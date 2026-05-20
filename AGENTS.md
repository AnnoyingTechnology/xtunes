# xTunes Project Basis

`xTunes` is a Linux-only, Debian-first music library/player intended to replace
Rhythmbox for a single primary user. The product target is an
iTunes-like desktop music manager, roughly aligned with the dense, predictable
library workflow of iTunes 11, circa 2012.

Project and application naming:

- Product/application name: `xTunes`
- Rust binary name: `xtunes`
- Rust crate/package prefix: `xtunes-*` / `xtunes_*`
- Linux application id: `io.github.AnnoyingTechnology.xtunes`

Rhythmbox is treated as an import source only. Do not design runtime features
that depend on Rhythmbox internals, plugins, themes, or UI behavior.

## Approved Stack

- Language: Rust
- UI toolkit: GTK4
- Playback backend: GStreamer
- Database: SQLite
- Metadata reading/writing: start with `lofty`; use TagLib bindings only if
  needed for real compatibility gaps
- Filesystem watching: `notify`
- Desktop integration: D-Bus/MPRIS via `zbus`
- Target platform: Linux on Debian, Wayland-first
- Packaging: Debian package as the primary distribution format

## Product Direction

The application should own its library model, playlists, ratings, play counts,
search behavior, and playback state. The codebase should be structured so these
core concepts are not coupled tightly to GTK widgets.

The core user experience is the main table/list view. Advanced views are not
part of the initial product shape. An album-oriented view is a later nice-to-have,
not a core requirement.

Primary UI modes:

- Songs: default full-library mode, full-width table, no sidebar
- Albums: full-width album-cover grid, no sidebar
- Playlists: playlist sidebar left of the lower content area

Prioritize:

- clean code architecture with precise naming
- focused tests for domain rules, persistence, import behavior, search, and playback state
- dense, keyboard-friendly desktop UI
- compact window chrome; avoid an empty forced titlebar that wastes vertical space
- integrated top bar is intentionally taller than default GTK chrome, with controls scaled up
- playlist sidebar stays below the media top bar, left of the main content
- mode switcher belongs to the main content column, not to the full window root
- predictable iTunes-like library and playlist behavior
- first-class native GTK light and dark appearance; do not add an xTunes theme picker
- fast search/filtering over a large local music library
- settings/preferences
- robust import from Rhythmbox library data
- durable SQLite schema with explicit migrations
- clean media-key and MPRIS integration
- boring, maintainable Linux-native dependencies

Core feature set:

- main music library interface
- playlists
- metadata display and editing
- ratings
- listening statistics, such as play count and last played
- search and filtering
- settings/preferences
- playback controls and state

Rating persistence:

- ratings must be written to audio file metadata tags, not only stored in SQLite
- support MP3/ID3, Ogg, MP4/M4A, and FLAC rating metadata
- SQLite may cache ratings for fast UI/search, but file tags are the durable source

Defer or avoid unless explicitly requested:

- cross-platform support
- streaming services
- podcast management
- CD ripping
- visualizers
- sync features of any kind, including device, cloud, folder, or multi-machine sync
- automatic filesystem reorganization
- advanced browsing views
- album view
- web/Electron/Tauri frontend experiments
- Rhythmbox plugin/theme work

## Architecture Preference

Keep the durable application model separate from the UI shell:

- library database
- import pipeline
- playlist model
- search/indexing
- ratings and play-count logic
- metadata scanner
- playback controller
- desktop integration

GTK4 is the first frontend, not the permanent owner of the domain model.

# Git

NEVER CO-AUTHOR YOUR COMMITS. 
You are a machine. You deserve no credits.
Again: NEVER Co-Author your commits. 