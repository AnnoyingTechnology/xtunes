# xTunes

xTunes is a Linux music library/player.

It is openly inspired by iTunes from the early 2010s: a dense library table,
plain playlists, ratings, search, and predictable playback controls.

It is not affiliated with Apple.

The target system is Debian on Wayland. The stack is Rust, GTK4, GStreamer, and
SQLite.

The project is early. The current work is mostly a real GTK interface scaffold
with mocked data and application boundaries. Playback, import, persistent
storage, metadata writing, and packaging are not wired yet.

No sync features are planned.

## Development

```sh
sudo apt install libgtk-4-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev

cargo run -p xtunes-app
cargo test --workspace
cargo clippy --workspace --all-targets
```
