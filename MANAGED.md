# Managed Library Plan

This document defines the safety and implementation contract for Sustain's
managed-library mode. The feature is intentionally treated as high risk:
Sustain will be taking responsibility for precious user-owned audio files.

The default product contract remains non-destructive. Sustain must not move,
rename, delete, or reorganize audio files unless the user has explicitly chosen
a managed-library workflow and confirmed any destructive follow-up.

## User-Facing Model

Preferences should expose two related controls under the library path:

- `Keep my library organized`
- `Organize Existing Library...`

`Keep my library organized` controls what happens to newly added tracks. It does
not reorganize anything that is already indexed.

`Organize Existing Library...` is a separate explicit workflow for tracks that
were previously indexed while Sustain was in non-managed mode. It should only be
available when:

- a library path is configured,
- `Keep my library organized` is enabled, and
- the managed-library foundation exists behind the UI.

Avoid the term `Consolidate` for this workflow. `PLAN.md` already reserves
duplicate consolidation for merging multiple library entries into one canonical
track. Good labels for this workflow are closer to:

- `Organize Existing Library...`
- `Copy Existing Files into Library...`
- `Manage Existing Files...`

## Safety Rules

These rules are not optional:

- Enabling the checkbox alone never moves existing files.
- New external files are copied into the library first; originals are not
  deleted as part of the copy operation.
- Original deletion is offered only after copy, verification, metadata scan, and
  SQLite/runtime update all succeed.
- The delete-originals prompt defaults to keeping originals.
- Sources already inside the managed library root are never offered for
  deletion.
- A partial failure cancels any delete-originals prompt.
- A failed copy or move leaves the library model pointing at the original valid
  file.
- Missing tracks are skipped and reported; they are not silently removed.
- Every file-taking path goes through one runtime command pipeline.

The shared command pipeline must cover:

- file chooser imports,
- Nautilus / `text/uri-list` file drops,
- folder drops,
- future Rhythmbox/importer flows,
- future `Organize Existing Library...`.

Do not implement parallel copy/move logic in GTK widgets.

## Domain Setting

Prefer an explicit enum over a loose boolean:

```rust
pub enum LibraryManagementMode {
    ReferenceFilesInPlace,
    CopyAddedFilesIntoLibrary,
}
```

The setting belongs under grouped library settings, not as a top-level ad hoc
field.

The meaning is:

- `ReferenceFilesInPlace`: added files are indexed at their current path.
- `CopyAddedFilesIntoLibrary`: external added files are copied under the library
  root using the managed path planner, and the in-library copy becomes the
  canonical track location.

## Path Planner

Build a pure, heavily tested path planner before filesystem work.

The exact visible pattern needs maintainer validation before implementation.
The intended first-pass shape is:

```text
Artist/Album/NN Title.ext
```

Open decisions:

- artist fallback: album artist, artist, composer, `Unknown Artist`, or another
  label
- album fallback: album title, `Unknown Album`, or no album folder
- title fallback: title tag, source filename stem, or `Untitled`
- track numbering: when to include `NN`, how to handle missing track number
- disc numbering: whether multi-disc albums use `D-NN Title.ext`
- compilations: whether artist appears in filename for multi-artist albums
- Unicode policy: preserve Unicode where possible, normalize only where needed
- forbidden characters: slash, NUL, control characters, path separators
- max filename/component length
- collision suffix format

Planner requirements:

- preserve the original extension,
- never produce an absolute path,
- never produce `..` components,
- never produce empty components,
- sanitize path separators and control characters,
- handle missing metadata deterministically,
- resolve collisions deterministically,
- return a structured plan, not just a string.

The planner should live below GTK, likely in domain or app-runtime depending on
whether it needs runtime services. The pure formatting/sanitization part should
be unit-testable without SQLite or real filesystem access.

## Duplicate Detection

Managed add should skip files already present in the library by content hash,
not by path.

This implies adding a track content hash to the library model/schema before
managed import becomes visible. During pre-release, edit the authoritative
schema directly and require a database wipe/rescan rather than adding migration
code.

Hashing rules:

- hash file content, not metadata fields,
- store enough hash metadata to explain duplicate decisions,
- treat hash failures as explicit import failures,
- avoid rehashing unchanged known files during normal rescans when size/mtime
  can prove nothing changed.

## Filesystem Transaction Shape

The safe copy path should be:

1. Resolve and validate the source path.
2. Read metadata from the source file.
3. Compute content hash.
4. Detect existing duplicate by hash.
5. Ask the path planner for the destination relative path.
6. Create destination directories.
7. Copy to a temporary file inside the destination directory.
8. Verify copied size and hash.
9. Atomically rename the temporary file to the final path.
10. Save/index the track using the final in-library relative path.
11. Update runtime state and UI from the persisted model.

If any step fails, clean up only temporary files created by Sustain and leave the
source file untouched.

Do not update SQLite/runtime state before the verified in-library file exists.

## Organize Existing Library Workflow

This workflow should reuse the same planner, duplicate detection, copy
primitive, and runtime command machinery as managed add.

The workflow should open with a preview/confirmation dialog before touching
files. The preview should at least report:

- number of tracks already managed,
- number of external tracks eligible to copy,
- number of missing tracks skipped,
- number of duplicates that would be skipped,
- number of destination collisions that will receive suffixes,
- estimated destination root.

First implementation can be all-or-nothing for the eligible batch. If any copy
or persistence step fails, do not offer original deletion.

Original deletion, if implemented, should be a second confirmation after the
successful copy/index pass. It must default to keeping originals.

## Background Work And Progress

Managed add and organize-existing-library can touch many files and must not run
on the GTK main thread.

Progress should be reported through the standard application status path, with
typed phases such as:

- planning,
- hashing,
- copying,
- verifying,
- indexing,
- finished,
- failed.

Cancellation should be supported before high-volume workflows are considered
complete. Cancelling should leave already completed copies/indexed tracks in a
consistent state and should never delete originals.

## Current Implementation Status

Implemented:

- `LibraryManagementMode` exists in domain settings and is persisted in TOML.
- Preferences surfaces `Keep my library organized` only when the library path is
  an existing directory.
- Track locations distinguish library-relative files from absolute external
  files, so reference mode can index dropped/imported files in place.
- Scans and explicit imports store SHA-256 content hashes for duplicate
  detection.
- The managed path planner is pure and covered by domain tests.
- The verified copy primitive copies to a temporary file, verifies size/hash,
  and refuses to overwrite an existing destination.
- `AddExternalLibraryItems` handles explicit file/folder additions:
  - reference mode indexes files in place,
  - managed mode copies files into the library and indexes the copy,
  - duplicate content hashes are skipped,
  - SQLite batch saves are transactional.

Still intentionally not implemented:

- Nautilus / `text/uri-list` drops onto Songs. Do not wire this directly to the
  synchronous runtime command; add a background import task first.
- Post-success "delete originals" prompting.
- `Organize Existing Library...`.
- User-editable path templates.

## Implementation Milestones

### Milestone 1: Foundation Only

- Done: add `LibraryManagementMode` to settings.
- Done: add path planner with tests.
- Done: add content hash model/schema support.
- Done: add safe copy primitive behind tests.
- Done: add runtime command shape for adding external files/folders.
- Done: keep the checkbox disabled until the path and backend support are safe.

### Milestone 2: Managed Add

- Done: implement managed add for explicit file/folder additions.
- Support Nautilus / `text/uri-list` drops onto Songs.
- Done: use copy-first semantics.
- Done: skip duplicates by hash.
- Report partial failures explicitly.
- Offer delete-originals only after full success.

### Milestone 3: Preferences UI

- Done: surface `Keep my library organized`.
- Done: add muted explanatory text under the checkbox.
- Done: make it clear that existing files are not reorganized automatically.
- Done: enable the setting only when a valid library path exists.

### Milestone 4: Organize Existing Library

- Add `Organize Existing Library...` behind managed mode.
- Show preview/confirmation before filesystem work.
- Reuse managed-add primitives.
- Keep original deletion as a separate post-success prompt.

## Test Requirements

Path planner tests:

- missing artist/album/title fallbacks,
- extension preservation,
- unsafe character sanitization,
- no absolute paths,
- no parent components,
- stable collision suffixing,
- long component handling,
- Unicode preservation/sanitization policy.

Runtime/filesystem tests:

- unmanaged add indexes the original file path,
- managed add copies to the planned relative path,
- managed add stores the copied file as canonical,
- duplicate hash is skipped and reported,
- copy failure leaves store/runtime unchanged,
- verification failure removes temporary files only,
- source files are never deleted during copy/index,
- toggling the setting never moves existing tracks.

Organize-existing tests:

- already-managed files are skipped,
- external existing files are copied and retargeted,
- missing tracks are skipped and reported,
- partial failure cancels delete-originals,
- playlist memberships, ratings, metadata cache, and statistics survive
  retargeting.

## Non-Goals For The First Pass

- No automatic reorganization when the checkbox is toggled.
- No duplicate-entry merge/consolidation.
- No destructive move-first behavior.
- No hidden GTK-only import path.
- No schema migrations during pre-release.
- No path-template language until the fixed first-pass planner has proven safe.
