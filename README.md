# xTunes

![xTunes interface screenshot](screenshot.png)

xTunes is a Linux music player for local music libraries.

> I was an Apple fanboy during the iTunes golden era (2005-2015). Each new
> release was a real treat, and this is the thing I missed most after switching
> to Linux. The advent of solid LLM agents in early 2026 allowed me to get
> a heavily inspired alternative rolling.
> I'm not cloning a specific version, instead picking from different generations.

Configure a library folder, let the app index it, and play.

It is not for podcasts, web radio, videos, streaming services, or device sync.

It is designed to work visually in both dark and light mode. Contrast, tint,
chrome, table rows, and controls are balanced in both themes, not tuned
for one and tolerated in the other.

The target system is Debian on Wayland. The stack is Rust, GTK4, GStreamer, and
SQLite.

The project is early. The current work is mostly a real GTK interface scaffold
with mocked data and application boundaries. Playback, import, persistent
storage, metadata writing, and packaging are not wired yet.

No sync features are planned. Import features from iTunes (.xml) or rhythmbox are likely.

## Development

```sh
sudo apt install libgtk-4-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev

cargo run -p xtunes-app
cargo test --workspace
cargo clippy --workspace --all-targets
```
