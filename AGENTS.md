# Sustain Project Basis

`Sustain` is a Linux-only, Debian-first music library/player intended to replace
Rhythmbox for a single primary user. The product target is an
iTunes-like desktop music manager, roughly aligned with the dense, predictable
library workflow of iTunes 11, circa 2012.

Project and application naming:

- Product/application name: `Sustain`
- Rust binary name: `sustain`
- Rust crate/package prefix: `sustain-*` / `sustain_*`
- Linux application id: `io.github.open_sustain.sustain`

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
- License: GPL-3.0-or-later (declared in `[workspace.package]`); do not relicense or add dependencies with incompatible licenses
- Every new `.rs` file starts with `// SPDX-License-Identifier: GPL-3.0-or-later` then `// Copyright (C) 2026 AnnoyingTechnology`

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
- first-class native GTK light and dark appearance; do not add an Sustain theme picker
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

- ratings must be written to audio file metadata tags IN ADDITION TO the SQLite db
- support MP3/ID3, Ogg, MP4/M4A, and FLAC rating metadata
- SQLite may cache ratings for fast UI/search, but file tags are the durable source

## Performance

Performance is a first-class feature, not a polish step. Target pristine
responsiveness and fluidity on a 10,000-track library: instant search,
smooth scrolling, snappy view switches, fast cold start. Code that ships
visibly sluggish behavior at that scale is incomplete, regardless of
correctness.

The maintainer develops on a Ryzen AI Max 395 — the top of the current
desktop performance range. Anything that feels (or measures) slow on
this machine will be worse on real-world hardware.

## Development Phase

Sustain is in pre-release development. It has never been published and has no
external users; the only databases, settings files, or on-disk artefacts that
exist are the maintainer's local working copies.

Practical consequences for anything stored on disk (SQLite schemas, settings
files, cached artwork, exported data, etc.):

- The on-disk format is **not** stable. Change the schema by editing the
  authoritative definition (e.g. the `CREATE TABLE` statements) directly.
- Do **not** add migration code, compatibility shims, column renames,
  `IF EXISTS` fallbacks, or "legacy path" normalisers. There is no legacy.
- New features may freely change the on-disk format. The expectation is that
  the maintainer wipes the local database and re-scans the library; that
  is cheaper and safer than carrying migration code for schemas that never
  shipped.
- Code that exists only to read or convert from a previous in-development
  schema must be removed, not kept "just in case".

A stable, migration-friendly schema lifecycle starts at the first public
release, not before. Until then, prefer deleting and recreating over
migrating.

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

>> EXTREMELY IMPORTANT <<<

NO HACKS. The user is EXTREMELY concerned about code quality, much more so than
immediate results. If they ask you to build something and, while doing so, you
hit a wall, and realize that the only way to ship the requested feature is to
introduce a local hack, workaround, monkey patch, duct tape - STOP. STOP
IMMEDIATELY. Either fix the underlying flaw that blocked you in a ROBUST, WELL
DESIGNED, PRODUCTION READY manner, or be honest that the prompt can't be
completed without hacks.

To make it very clear:

- DO NOT INTRODUCE HACKS IN THE CODEBASE.
- DO NOT COMMIT CODE THAT COULD BREAK THINGS LATER.
- DO NOT COMMIT PARTIAL SOLUTIONS OR WORKAROUNDS.

THIS IS VERY IMPORTANT.
THIS IS VERY IMPORTANT.
THIS IS VERY IMPORTANT.

The author appreciates honestly and he WILL be glad and thankful if you respond
a request with "I couldn't complete your request because the repository lacked
support for X". He WILL be even happier if you go ahead and update the repo to
provide the necessary support in a well designed, robust way. But he will be
VERY ANGRY if, while attempting to implement a feature, you introduce a
workaround that will potentially break things later.

NEVER introduce hacks in the codebase.

Also assume that none of the code you're working in is in production, so,
backwards compatibility is NOT IMPORTANT. If you find something that is poorly
designed and fixing it would require breaking existing APIs or behavior, DO SO.
Do it properly rather than preserving a flawed design. Prioritize clarity,
correctness, and maintainability over compatibility with existing code.

Core values:
- ABSOLUTE code quality over speed of delivery.
- Correctness over convenience.
- Clarity over cleverness.
- Maintainability over short-term productivity.
- Robust design over quick fixes.
- Simplicity over complexity.
- Doing it right over doing it now.
- Honesty above everything.

After every change you make, provide a clear, honest report on ANY change that
you are not confident about and that could be considered a fragile hack.