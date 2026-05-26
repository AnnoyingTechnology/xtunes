# Bugs Backlog

## Artwork fetch resets song table scroll position

Clicking a missing-artwork cell to trigger artwork retrieval scrolls the
song table back to the top. Expected: scroll position is preserved while
the artwork fetch completes and the row updates in place.

## Now-playing artwork lags behind track info on Next

Clicking Next updates the now-playing track info (title, artist, etc.)
immediately, but the artwork in the now-playing space pops in roughly
500 ms later. Looks laggy; artwork should update in the same frame as
the rest of the track info.

## Tag edit from file info window scrolls table to top

Editing a tag from the file info window and clicking OK scrolls the
song table back to the top. Only the affected row should redraw;
scrolling to the top is unacceptable.

## Seek cursor stutters while held still mid-drag

While actively moving the seek cursor, stuttering is expected. But if
the drag is paused — mouse button still held, cursor stopped at a
position — playback continuously stutters instead of playing cleanly
from that position. Stutter should only occur while the cursor is
actually moving.

## Main window shadow looks truncated

The drop shadow around the main window appears cooked — it looks
clipped by a square box around the window, as if the shadow's render
region is smaller than the shadow itself. Possibly the shadow is too
large for whatever surface is compositing it. Needs investigation.

## Album Play button should disable shuffle

In the expanded album view, clicking the Play button should force
sequential playback — if shuffle is currently on, it must be turned
off before the album starts. The expanded view already exposes a
separate Shuffle play button for the shuffled case, so the plain
Play button is unambiguous: play this album in order, from the
first track.

## Dates displayed in UTC instead of local timezone

The "Added Date" column (and likely other timestamps) is rendered in
UTC rather than the system's local timezone. In Paris (CEST, UTC+2)
the displayed time is 2h behind the actual local time. All
user-facing timestamps should be converted to the local timezone for
display; storage in UTC is fine, but the formatter must apply the
local offset.

## Albums view has top/bottom padding clipping the grid

The artwork grid in the Albums view has top and bottom padding inside
its scroll container. When scrolling, the grid does not flow all the
way to the edges of the container — covers get cut off by the padding
instead of fading naturally at the scroll bounds. The grid should
extend edge-to-edge vertically so content scrolls cleanly under the
top/bottom of the viewport.

## Persist UI state across sessions

Not a bug, parked here. On close we should save and restore:

- the current search text, if any
- the current view (Songs, Albums, Playlists)
- if in Playlists, which playlist was selected

Storage location is open — TOML (settings) or SQLite (library db),
whichever fits best. Leaning toward TOML since this is UI/session
state rather than library data, but to be decided at implementation.
