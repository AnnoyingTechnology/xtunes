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
