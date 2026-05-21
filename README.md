# xTunes

![xTunes interface screenshot dark](screenshot-dark.png)

![xTunes interface screenshot dark](screenshot-light.png)

Open xTunes is a Linux music player for local music libraries.

> I was an Apple fanboy during the iTunes golden era (2005-2015).
> Each new release was a real treat, and this is the thing I missed most after switching to Linux.
> Advent of solid LLM agents in late 2025 allowed me to get a substitute rolling.

This player is not a true iTunes clone for three reasons :
- with each versions good aspects came and went, I'm cherry picking what I believe to be tasteful,
- lots of features that were added over time were pure bloat,
- not to infringe on intellectual property.

This player is not designed by commity nor is it it's purpose. It's opinionated and autoritarian, as most of Apple's good products were.

The interface is working natively in both Light and Dark modes. It also leverages natives aspects of Gnome to get the right blend of bespoke visual component without bending GTK or GNome excessively.
It is an affair of compromise. Native _icons_ are used, native _accent color_ is used, etc.

The target system is Debian on Wayland. The stack is Rust, GTK4, GStreamer, and
SQLite. I'm striving for fast, safe and robust code.

Features that will _probably_ come later :
- Import from iTunes/Apple Music (.xml)
- Import from Rhythmbox
- Encode a CD
- Convert a file to MP3 320 or Flac
- Sync to Android

## Key locations

- Config: `~/.config/xtunes/settings.toml`
- Database: `~/.local/share/xtunes/library.sqlite`

## Development

```sh
sudo apt install libgtk-4-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev

cargo run -p xtunes-app
cargo test --workspace
cargo clippy --workspace --all-targets
```

### Sidenote

A longtime friend who stayed on macOS told me iTunes (now Apple Music) had "lost the plot" and that he'd love a version without all the junk. He also mentioned the latest Apple Music puts the player at the bottom of the window. I didn't believe him, turns out it's true.
Apple has lost its way, but 2010-era Apple nailed music playback. So there's probably room for a deshitified iTunes on macOS too. That could come at a later stage.