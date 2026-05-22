# Bugs

Open bugs in the track-context-menu work. Each section is self-contained for
a fresh agent. Files referenced are at the workspace root paths shown.

---

## 1. Context-menu popover renders white-on-white in the Songs / Playlists table

### Symptom

Right-clicking a row in the Songs view (and Playlists view, which shares
`build_track_table`) opens a `gtk::PopoverMenu` whose menu items are white
text on a near-white background — unreadable. In the Albums view the same
menu renders correctly.

### Where it lives

- `crates/ui_gtk/src/track_context.rs` → `TrackRowContextMenu::popup_at` is
  what `set_parent`s the popover to the right-clicked widget.
- `crates/ui_gtk/src/track_table.rs` → `install_cell_context_menu` is what
  calls `popup_at(track_ids, &cell_for_gesture, x, y)`. The `cell_for_gesture`
  is the cell `gtk::Box` carrying the `.track-table-cell` class (and
  `.track-table-row-selected` when its row is selected).
- `crates/ui_gtk/src/lib.rs` → `install_app_css` is the app's CSS provider.

### Root cause

GTK's own theme styles `listview > row:selected` with the selected foreground
color. `color` inherits, so the cascade row → cell-widget → our cell box →
the popover parented to that cell box delivers selected-fg to the popover's
menu items. In Albums the anchor widget is a plain `gtk::Label` outside any
`:selected` scope, so the cascade is benign — which is why only Songs is
affected.

Adwaita normally has a `popover > contents { color: @theme_fg_color; }` rule
that would reset this for popovers. In this app it isn't winning the cascade,
either because the user's theme version (GTK 4.22) doesn't have it or because
its specificity loses to the row rule. **Do not** "fix" this with `!important`
or higher-specificity overrides. The project is explicitly GNOME light/dark
auto-mode compliant — fighting the theme is out of scope.

### What was already tried (failed)

- Adding `popover.menu { color: @theme_fg_color; }` and broader variants
  (`popover.menu modelbutton label { color: … }`) in `install_app_css`. Did
  not visibly affect rendering.
- Restructuring `install_app_css` so selected foreground color is only set on
  `> label, > image` direct children of the cell box, not the box itself.
  Did not help — GTK's own `row:selected` color cascade dominates regardless.
- Re-parenting the popover to the `gtk::ColumnView` ancestor (out of the
  `:selected` cascade scope), translating coordinates accordingly. This
  silently broke popup activation in Songs — right-click produced no visible
  menu at all. The failure mode of `popover.set_parent(&column_view)` was
  never diagnosed and may be a GTK quirk specific to `ColumnView`.

### Suggested next step

The structural fix — parent the popover to a widget *outside* the `:selected`
row scope — is sound; only the chosen parent failed. Likely better targets:

- The `gtk::ScrolledWindow` that wraps the `ColumnView` (a plain container).
- The main `gtk::Window` (also already accessible — already threaded into
  `TrackRowContextMenu::new` for the dialog parent).

Whichever target is chosen, `set_pointing_to` needs a rectangle in that
target's coordinate space — use `anchor.translate_coordinates(&new_parent,
x, y)`. The albums view's `install_track_row_context_menu` should keep its
current behavior (label anchor); the issue is Songs-specific.

Open GTK Inspector (Ctrl+Shift+I if GTK_DEBUG enables it, or via the GNOME
debugging shortcut) before deciding the new parent, to confirm the reparented
popover actually realizes.

---

## 2. "Move to Trash" produces no confirmation dialog; track is not trashed

### Symptom

Right-click → Move to Trash on a row. Expected: `gtk::AlertDialog` modal with
Cancel / Move to Trash buttons. Actual: no dialog appears and the track is
not trashed — the action is dead.

(Earlier in the session, before `glib::idle_add_local_once` was introduced,
the Albums view showed a different failure: the track was trashed
immediately, no dialog, suggesting `dialog.choose` returned synchronously
with the default-button index. That symptom has since shifted.)

### Where it lives

- `crates/ui_gtk/src/track_context.rs` →
  - `TrackRowContextMenu::new` wires the trash action; the handler reads the
    selected `Vec<TrackId>`, then `glib::idle_add_local_once`s a closure that
    calls `confirm_move_to_trash`.
  - `confirm_move_to_trash` builds the `AlertDialog`, calls `.choose(parent,
    None, callback)`, and only invokes `on_confirm` on `Ok(CONFIRM_BUTTON_INDEX)`.
- `crates/ui_gtk/src/lib.rs` → `build_main_window` threads the application's
  `gtk::Window` into `TrackRowContextMenu::new` so the dialog parent is
  always a real, stable window.
- `Cargo.toml` → `gtk = { package = "gtk4", version = "0.10", features =
  ["v4_10"] }`. The `v4_10` feature is required for `gtk::AlertDialog`.

### Diagnostic instrumentation already in place

`track_context.rs` contains `eprintln!("[xtunes][ctxmenu] …")` calls at every
step:
- "trash action fired with N ids"
- "idle fired — building dialog"
- "confirm_move_to_trash: parent=…, message=…"
- "AlertDialog.choose callback fired with result: …"
- "dialog.choose returned (async pending)"
- "on_confirm running (N ids)"

The stderr from a real right-click → Move to Trash has not been captured.
**First diagnostic step is to capture this** — `cargo run 2>&1 | grep xtunes`,
then right-click → Move to Trash and read which prints appear:

- No prints at all → the action isn't firing. The popover's menu-item →
  action group resolution is broken. Check `popover.insert_action_group(…)`
  in `popup_at` and whether the action group is reachable from the menu
  model's `"track-context.move-to-trash"` reference.
- "trash action fired" only → the `idle_add_local_once` source isn't
  executing. Verify the returned `SourceId` lifetime (`glib::SourceId`
  semantics in glib-rs 0.20 should keep `_once` sources alive on drop, but
  confirm).
- Idle fires, `dialog.choose returned` prints, then the choose callback
  prints almost immediately with `Ok(1)` → the dialog is closing
  synchronously with the default-button index without showing. Likely
  Wayland-related; verify the parent window is mapped / focused.
- Idle fires, choose returns, callback never prints → the async never
  completes. Dialog object lifetime issue or display surface issue.

### Cleanup required after fix

All `eprintln!("[xtunes][ctxmenu] …")` calls in
`crates/ui_gtk/src/track_context.rs` are diagnostic only and **must be
removed** once the dialog flow is verified working.

---

## Pre-existing deprecation warnings (cleanup, not bugs)

Surfaced when `v4_10` was enabled on `gtk4` for `AlertDialog`. They were
latent — the call sites had been using APIs already deprecated since
GTK 4.10. Runtime behavior is unaffected. Tracked here so they're not lost:

- `crates/ui_gtk/src/preferences.rs:228+` — `gtk::FileChooserNative` (whole
  builder chain). Migrate to `gtk::FileDialog`.
- `crates/ui_gtk/src/albums.rs:522` — `area.style_context().color()`.
  Replace with `area.color()` (or equivalent on the current widget).
- `crates/ui_gtk/src/now_playing.rs:577` — `canvas.style_context().color()`.
  Same replacement.
- `crates/ui_gtk/src/albums.rs:560-561` —
  `widget.style_context().add_provider(provider, priority)` for the per-album
  dynamic palette CSS. **Non-trivial** — the replacement requires either
  moving the dynamic provider to the display and scoping with unique
  per-instance CSS classes, or applying palette colors as inline widget
  properties. Touches `album_detail_palette_provider`, `apply_palette_style`,
  and the artwork-color flow.
