# Managed Library Plan

This document defines the safety contract for Sustain's managed-library mode. The
feature is high risk because Sustain takes responsibility for user-owned audio
files. The implementation must prefer correctness and recoverability over a
convenient UI shortcut.

## Product Model

Preferences exposes one control under the library path:

- `Keep my library organized`

When disabled, Sustain indexes files already inside the configured library folder
and leaves their relative paths alone.

When enabled, Sustain owns the library layout:

- newly added files are copied into the managed artist/album/track layout,
- existing indexed files are reorganized automatically in the background,
- every canonical track path remains relative to the configured library root,
- files outside the library root are never stored as canonical track locations.

There is no separate `Consolidate` or `Organize Existing Library` button. The
iTunes-like behavior is that enabling the setting starts organization work in
the background.

## Automatic Organization Semantics

Enabling `Keep my library organized` starts a background library organization
task after the setting has been saved.

Disabling it while that task is running requests cancellation. Cancellation is
cooperative: the current file move is allowed to finish, already moved tracks
remain valid, and no further tracks are moved after the task observes the
request. This is intentional so an accidental enable can be stopped before the
whole library is reorganized.

Disabling the setting later does not move files back. It only changes behavior
for future additions.

## Safety Rules

These rules are not optional:

- Track locations are library-relative only.
- Managed organization never copies existing in-library files to reorganize
  them. It performs metadata-only same-filesystem moves.
- The move primitive must refuse destination overwrite.
- Cross-device moves fail; they must not fall back to copy/delete.
- Missing tracks are skipped and reported.
- Path planning for the batch happens before touching files.
- Playlist membership, track IDs, ratings, metadata, and statistics are
  preserved by retargeting existing tracks rather than creating new ones.
- No GTK widget may implement its own file move/copy path.
- Scan, import, and organization tasks are mutually exclusive.
- Library path changes and manual scans are blocked while a library task is
  running.
- The managed-mode checkbox may be unticked during organization; that is the
  cancellation control.

## Filesystem Move Shape

Rust `std::fs::rename` overwrites existing destinations on Unix, which is not
acceptable for precious audio files. The first implementation therefore uses a
same-filesystem metadata move implemented as:

1. Require the source to be a regular file.
2. Refuse an existing destination.
3. Create the destination directory.
4. Create a hard link at the destination path.
5. Remove the old source path.

This does not copy file contents and does not hammer SSDs. It also gives
atomic no-overwrite destination creation. If the filesystem does not support
hard links or the paths are on different devices, the move fails instead of
falling back to a copy.

## Recovery Journal

Filesystem moves and SQLite updates are not one atomic transaction. Managed
organization must therefore write a small journal in the library root before
moving files.

The journal contains the planned track ID, source relative path, and destination
relative path for each move. On startup, and before a new organization task,
Sustain reconciles the journal:

- destination exists and source does not: update SQLite to the destination,
- source and destination exist as the same file: remove the old source name and
  update SQLite to the destination,
- source exists and destination does not: leave the track at the source,
- source and destination both exist but are different files: delete nothing and
  leave the current database record untouched.

The journal is removed only after the task finishes or cancels with all completed
moves reflected in SQLite.

## Path Planner

Managed paths are planned by the pure domain planner.

The current first-pass layout is:

```text
Artist/Album/NN Title.ext
```

Planner requirements:

- preserve the original extension,
- never produce an absolute path,
- never produce `..` components,
- never produce empty components,
- sanitize path separators and control characters,
- handle missing metadata deterministically,
- resolve collisions deterministically,
- return structured plans.

Open product decisions remain:

- exact artist and album fallback wording,
- multi-disc filename style,
- compilation handling,
- user-editable path templates.

## Duplicate Detection

Managed add skips external files already present in the library by content hash.

Normal library scans must not hash file contents. During managed import, existing
tracks without hashes are checked lazily by file size first, then hashed only
when size matches an incoming file.

Automatic organization of existing files does not deduplicate tracks. It moves
the existing indexed track records to their managed paths while preserving their
identity.

Path-affecting metadata edits also participate in managed organization. When the
user edits artist, album artist, composer, album, title, track number, disc
number, disc total, or compilation status while managed mode is enabled, Sustain
plans a new managed path and moves the existing file there if the path changes.

## Current Implementation Status

Implemented:

- `LibraryManagementMode` exists in domain settings and is persisted in TOML.
- Preferences surfaces `Keep my library organized` only when the library path is
  an existing directory.
- Track locations remain library-relative.
- Managed external import copies files into the library with verified hashes.
- Normal scans do not hash file contents.
- Existing scanned tracks without hashes are lazily checked during managed
  import duplicate detection.
- The managed path planner is pure and covered by domain tests.
- Nautilus / `text/uri-list` drops onto Songs use the runtime import pipeline.

Required next step:

- Automatic background organization when `Keep my library organized` is enabled.

Non-goals for this step:

- No duplicate-entry merge/consolidation.
- No moving files back when managed mode is disabled.
- No user-editable path templates.
- No schema migrations during pre-release.
