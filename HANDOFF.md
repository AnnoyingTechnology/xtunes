# HANDOFF — fix track context menu regressions

This is a temporary handoff doc. Delete after the task is complete.

## What the user reported (three issues)

After landing the right-click "Move to Trash" / "Remove from Library" context
menu (shared between the Songs table and the Albums view), the user ran the
app and found:

1. **White-on-white text in the Songs-table context menu.** The popover menu
   renders unreadable in the Songs table (and Playlists table, which uses
   the same `build_track_table`). The same menu in the Albums view looks
   normal.
2. **Missing confirmation dialog.** Move-to-Trash must prompt the user with
   "the cleanest, best-sanctioned way" to confirm. Currently the action
   trashes immediately with no prompt.
3. **No multi-row selection.** The track table only allows selecting one
   row. The user wants standard desktop semantics — Ctrl-click toggles,
   Shift-click range-selects.

Direct user quote:
> "I don't know what happened, but now the contextual menu in song table is
> white text on white background. While it's still normal in albums view.
> The contextual menu shoudl behave similarily in both wtf. Also, the
> trashing SHOULD prompt using the cleanest, best sanctionned way to
> confirm. Also, we should be able to select multiple rows (in general).
> using ctrl or shift click, etc."

## Root cause of issue #1 (white-on-white)

The shared `TrackRowContextMenu` lives in
`crates/ui_gtk/src/track_context.rs`. Its `popup_at` calls
`popover.set_parent(anchor)`, where `anchor` is the right-clicked widget.

- In the **Songs table**, the anchor is a cell `gtk::Box` that lives inside
  `columnview.track-table`. When the row becomes selected, the cell box
  gets the class `.track-table-row-selected`, and CSS in
  `install_app_css` (`crates/ui_gtk/src/lib.rs`) sets:
  ```css
  .track-table-cell.track-table-row-selected {
      background-color: @theme_selected_bg_color;
      color: @theme_selected_fg_color;
  }
  ```
  `color` is inherited. The popover is parented to that selected cell, so
  every label/modelbutton inside the popover inherits the selected
  foreground color — which is white on the default popover background.

- In the **Albums view**, the anchor is a `gtk::Label` with no
  `color`/`background` rules cascading onto it, so the popover renders
  normally.

## Fix plan

Three concrete changes, each independent enough to commit in one sweep.

### Fix #1 — Reset popover foreground

In `install_app_css` (`crates/ui_gtk/src/lib.rs`) add a rule that scopes
popover menu colors back to the theme defaults so the popover never
inherits foreground/background from whatever widget triggered it:

```css
popover.menu,
popover.menu > contents {
    color: @theme_fg_color;
    background-color: @theme_bg_color;
}
```

This is the lightest possible fix and works regardless of which widget the
popover is parented to. (We do NOT need to change `set_parent` to a
different anchor — the CSS reset is cleaner and addresses the root cause
that popover content should never inherit from its trigger widget.)

### Fix #2 — Confirmation dialog for Move to Trash

Use `gtk::AlertDialog` — the GTK 4.10+ sanctioned async API. Verified
available in our `gtk4 = 0.10.3` (the `AlertDialog::builder()` and
`.choose()` methods are unconditional; only the `notify_*` signals are
gated on the `v4_10` feature, which we don't need).

Wire it in `crates/ui_gtk/src/track_context.rs`:

- Store the last anchor's root window so the action handler can reach a
  parent window for the modal dialog:
  ```rust
  last_anchor_window: Rc<RefCell<Option<gtk::Window>>>,
  ```
  Update it in `popup_at`:
  ```rust
  self.last_anchor_window.replace(
      anchor.as_ref().root().and_then(|r| r.downcast::<gtk::Window>().ok())
  );
  ```
- In the trash action's `connect_activate`, instead of invoking the
  callback directly, build an `AlertDialog` and call `.choose(parent, ...)`
  with a callback that runs the trash on confirm (index 1) and does
  nothing on cancel (index 0).

Dialog copy (singular + plural):

- 1 track: message `"Move \"{title}\" to the trash?"`, detail `"The audio
  file will be removed from the library and moved to the system trash."`
- N tracks: message `"Move {N} tracks to the trash?"`, detail same as
  above with "files" plural.

(If "title" is awkward to thread through, use `"Move this track to the
trash?"` — the user said "cleanest, best sanctioned"; explicit titles are
nicer but not required.)

Buttons: `["Cancel", "Move to Trash"]`, `default_button=1`,
`cancel_button=0`, `modal=true`.

### Fix #3 — Multi-row selection

Switch `gtk::SingleSelection` → `gtk::MultiSelection` in
`crates/ui_gtk/src/track_table.rs` (`build_track_table` around line 352).
Ctrl/Shift click semantics come for free from `MultiSelection`.

Change `TrackActionCallback` from `Fn(TrackId)` to `Fn(Vec<TrackId>)` —
the action operates over the entire selection. This is in
`crates/ui_gtk/src/track_context.rs` lines 8–14.

Right-click semantics (mimic Files/iTunes):

- If the right-clicked row is NOT in the current selection: replace
  selection with just that row, then act on `[that_id]`.
- If the right-clicked row IS in the current selection: preserve the
  multi-selection and act on all of it.

Implement in the `install_cell_context_menu` helper
(`crates/ui_gtk/src/track_table.rs:424`):

```rust
gesture.connect_pressed(move |_gesture, _n_press, x, y| {
    let position = list_item.position();
    if position == gtk::INVALID_LIST_POSITION {
        return;
    }
    if !context.selection.is_selected(position) {
        context.selection.unselect_all();
        context.selection.select_item(position, false);
    }

    let track_ids = collect_selected_track_ids(&context.selection);
    if track_ids.is_empty() {
        return;
    }

    context.menu.popup_at(track_ids, &cell_for_gesture, x, y);
});
```

Where `collect_selected_track_ids` iterates `selection.selection()`
(a `gtk::Bitset`) and pulls `TrackId`s out via `row_track_id(selection.item(i))`.

Albums view (`crates/ui_gtk/src/albums.rs:38`,
`install_track_row_context_menu`) — there's no multi-selection there, so
just pass `vec![track_id]` to `popup_at`.

Update `TrackContextCallbacks`:
```rust
pub(crate) struct TrackContextCallbacks {
    pub(crate) remove_from_library: TrackActionCallback,
    pub(crate) move_to_trash: TrackActionCallback,
}
pub(crate) type TrackActionCallback = Rc<dyn Fn(Vec<TrackId>)>;
```

Update lib.rs `track_mutation_callback` to loop the command over the
incoming `Vec<TrackId>` (and only call `playback_changed` /
`library_changed` once at the end, not per id):

```rust
fn track_mutation_callback(
    runtime: &SharedRuntime,
    playback_changed: PlaybackChangedCallback,
    library_changed_holder: LibraryChangedHolder,
    command_builder: impl Fn(TrackId) -> ApplicationCommand + 'static,
) -> TrackActionCallback {
    let runtime = runtime.clone();
    Rc::new(move |track_ids: Vec<TrackId>| {
        let mut changed = false;
        for track_id in track_ids {
            let result = runtime
                .borrow_mut()
                .handle_command(command_builder(track_id));
            if result.is_ok() {
                changed = true;
            }
        }
        if !changed {
            return;
        }
        playback_changed();
        if let Some(callback) = library_changed_holder.borrow().as_ref() {
            callback();
        }
    })
}
```

Store `Rc<RefCell<Vec<TrackId>>>` in `TrackRowContextMenu` (replacing
`Rc<Cell<Option<TrackId>>>`).

### Menu labels (small UX nuance)

Keep "Remove from Library" and "Move to Trash" static — iTunes does the
same and it reads fine. The plural count only matters in the confirmation
dialog message (already covered above).

## Files involved

- `crates/ui_gtk/src/track_context.rs` — shared menu module. Biggest
  change: API moves from `TrackId` to `Vec<TrackId>`; trash action
  intercepts and shows AlertDialog.
- `crates/ui_gtk/src/track_table.rs` — switch to `MultiSelection`; right-
  click handler builds the id list; helper `collect_selected_track_ids`.
- `crates/ui_gtk/src/albums.rs` — `install_track_row_context_menu` passes
  `vec![track_id]`.
- `crates/ui_gtk/src/lib.rs` — `install_app_css` gets popover reset rule;
  `track_mutation_callback` accepts `Vec<TrackId>`.

## Verification checklist

After implementing:

1. `cargo build` and `cargo test` from workspace root must pass.
2. (User-run) Launch the app and confirm:
   - Songs table right-click: popover menu renders with normal theme
     colors (not white-on-white).
   - Albums view right-click: still renders normally (regression check).
   - Ctrl-click selects multiple rows; Shift-click selects a range.
   - Right-clicking a non-selected row replaces selection with that row.
   - Right-clicking inside a multi-selection preserves it.
   - "Move to Trash" pops a modal confirmation. Cancel does nothing.
     Confirm moves all selected files to trash and removes from library.
   - "Remove from Library" still removes immediately (no confirmation —
     user did not ask for one on that action; only trash needs prompt).

## Project guardrails (from CLAUDE.md)

- No hacks, no workarounds. If something can't be done cleanly, stop and
  report rather than ship a fragile fix.
- No backwards-compat shims; this is pre-release.
- Never co-author commits.
- Don't commit unless explicitly asked.
