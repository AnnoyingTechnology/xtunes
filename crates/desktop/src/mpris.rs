// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! MPRIS 2.2 server.
//!
//! Wayland desktops (and most modern Linux DEs) do not deliver the XF86
//! media keys to applications as keyboard events. The compositor catches
//! them and dispatches `Play`/`Pause`/`Next`/`Previous` over D-Bus to
//! whichever process implements the standard MPRIS interfaces at
//! `/org/mpris/MediaPlayer2`. Without an MPRIS server, an application is
//! invisible to the media-key bridge regardless of focus or GTK keyboard
//! shortcuts.
//!
//! ## Threading model
//!
//! zbus is async-first, but the rest of Sustain runs the domain model on
//! the GTK main thread inside `Rc<RefCell<…>>`. Mixing the two requires a
//! handoff:
//!
//! * A dedicated `sustain-mpris` thread runs `async_io::block_on` driving
//!   the zbus connection. It is the only thread that ever touches the bus.
//! * Inbound method calls (delivered by zbus on the worker thread) invoke
//!   the caller-supplied [`MprisPlaybackSink`] closure. The UI side wires
//!   that closure to push a [`MprisCommand`] into a cross-thread channel
//!   that a `glib::MainContext::spawn_local` task drains on the main
//!   thread, where it is safe to touch the runtime.
//! * Outbound state updates from the main thread come in through
//!   [`MprisService::publish_playback_state`] /
//!   [`MprisService::publish_now_playing`]. They are forwarded to the
//!   worker thread through an internal unbounded channel; the worker
//!   mutates the interface state and emits `PropertiesChanged`.
//!
//! ## Shutdown
//!
//! [`MprisService`] holds the only sender for the outbound channel.
//! Dropping the service closes the channel, which makes the worker's
//! `recv().await` return `Err`, which exits the loop, which drops the
//! zbus connection — releasing the bus name. The drop also joins the
//! worker thread so the bus name is gone by the time the call returns.

use std::{
    collections::HashMap,
    sync::Arc,
    thread::{self, JoinHandle},
};

use async_channel::Sender;
use sustain_app_runtime::{PlaybackState, TrackId};
use zbus::{
    connection, interface,
    object_server::InterfaceRef,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use crate::{DesktopError, DesktopResult, NowPlayingMetadata};

/// Well-known bus name. The MPRIS 2.2 spec recommends
/// `org.mpris.MediaPlayer2.<player>` where `<player>` is a short, stable
/// application identifier. We deliberately use the short `sustain` name
/// rather than the full reverse-DNS app id (`io.github.open_sustain.sustain`)
/// because that is what every other Linux music player ships and what
/// `playerctl`/`gnome-shell` autocomplete against. Multi-instance
/// disambiguation per the spec (`.instance<pid>` suffix) is deferred
/// until the single-instance enforcement work lands.
const BUS_NAME: &str = "org.mpris.MediaPlayer2.sustain";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const DESKTOP_ENTRY: &str = "io.github.open_sustain.sustain";
const IDENTITY: &str = "Sustain";

/// The path used for `mpris:trackid` when no track is currently active.
/// MPRIS 2.2 reserves `/org/mpris/MediaPlayer2/TrackList/NoTrack` for
/// this purpose; some clients (e.g. plasma-applets) misbehave if the
/// trackid is missing or empty rather than this sentinel.
const TRACK_ID_NONE: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// Inbound command from the bus. Translated by the UI side into the
/// appropriate `PlaybackCommand` / GTK action; the desktop crate stays
/// agnostic of what `Play` vs `Resume` means in the domain model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MprisCommand {
    Raise,
    Quit,
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
}

/// Callback invoked on the MPRIS worker thread whenever the bus delivers
/// a method call. Implementations MUST be cheap and non-blocking — they
/// run inside the zbus async runtime — and MUST be safe to call from a
/// thread that does not own the GTK runtime. The typical pattern is to
/// push the command into a thread-safe channel and let a main-thread
/// task drain it.
pub struct MprisPlaybackSink(Box<dyn Fn(MprisCommand) + Send + Sync>);

impl MprisPlaybackSink {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(MprisCommand) + Send + Sync + 'static,
    {
        Self(Box::new(callback))
    }

    fn invoke(&self, command: MprisCommand) {
        (self.0)(command);
    }
}

pub struct MprisStartConfig {
    pub command_sink: MprisPlaybackSink,
}

pub struct MprisService {
    update_tx: Sender<MprisUpdate>,
    worker: Option<JoinHandle<()>>,
}

impl MprisService {
    pub fn start(config: MprisStartConfig) -> DesktopResult<Self> {
        let (update_tx, update_rx) = async_channel::unbounded::<MprisUpdate>();
        let (startup_tx, startup_rx) = async_channel::bounded::<DesktopResult<()>>(1);
        let command_sink = Arc::new(config.command_sink);

        let worker = thread::Builder::new()
            .name("sustain-mpris".to_owned())
            .spawn(move || {
                async_io::block_on(async move {
                    let connection = match build_connection(command_sink).await {
                        Ok(connection) => {
                            let _ = startup_tx.send(Ok(())).await;
                            connection
                        }
                        Err(error) => {
                            let _ = startup_tx
                                .send(Err(DesktopError::BusConnectionFailed(error)))
                                .await;
                            return;
                        }
                    };

                    let player_ref: InterfaceRef<PlayerInterface> =
                        match connection.object_server().interface(OBJECT_PATH).await {
                            Ok(player_ref) => player_ref,
                            Err(error) => {
                                eprintln!("sustain MPRIS: lost player interface handle: {error:?}");
                                return;
                            }
                        };

                    while let Ok(update) = update_rx.recv().await {
                        if let Err(error) = apply_update(&player_ref, update).await {
                            eprintln!("sustain MPRIS: property push failed: {error:?}");
                        }
                    }

                    // The outbound channel was closed (service dropped).
                    // Releasing `connection` here drops the bus name.
                    drop(connection);
                });
            })
            .map_err(DesktopError::ThreadSpawnFailed)?;

        // Wait for the worker to finish its bootstrap so a startup error
        // (bus unavailable, name taken) surfaces as a failed `start()`
        // call rather than as a silent never-fires service. The channel
        // is bounded(1), so this blocks at most one round-trip on the
        // worker thread.
        match async_io::block_on(startup_rx.recv()) {
            Ok(Ok(())) => Ok(Self {
                update_tx,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                // The worker exited before reporting startup status.
                // Treat as a generic bus failure.
                let _ = worker.join();
                Err(DesktopError::BusConnectionFailed(zbus::Error::Failure(
                    "MPRIS worker exited during startup".to_owned(),
                )))
            }
        }
    }

    pub fn publish_playback_state(&self, state: PlaybackState) {
        // Unbounded channel: send only fails when the channel is closed,
        // i.e. the worker thread has exited. In that case the silent drop
        // is intentional — there is no point in surfacing it to the UI
        // and a panic here would crash the app over a missing media-key
        // bridge.
        let _ = self.update_tx.try_send(MprisUpdate::PlaybackState(state));
    }

    pub fn publish_now_playing(&self, metadata: NowPlayingMetadata) {
        let _ = self.update_tx.try_send(MprisUpdate::NowPlaying(metadata));
    }
}

impl Drop for MprisService {
    fn drop(&mut self) {
        // Closing the sender wakes the worker's `recv().await` with Err,
        // exits the loop, drops the connection (releasing the bus name),
        // and lets `block_on` return.
        self.update_tx.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
enum MprisUpdate {
    PlaybackState(PlaybackState),
    NowPlaying(NowPlayingMetadata),
}

async fn build_connection(command_sink: Arc<MprisPlaybackSink>) -> zbus::Result<zbus::Connection> {
    let root = RootInterface {
        command_sink: Arc::clone(&command_sink),
    };
    let player = PlayerInterface::new(command_sink);
    connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, root)?
        .serve_at(OBJECT_PATH, player)?
        .build()
        .await
}

async fn apply_update(
    player_ref: &InterfaceRef<PlayerInterface>,
    update: MprisUpdate,
) -> zbus::Result<()> {
    match update {
        MprisUpdate::PlaybackState(state) => {
            let new_status = playback_status_text(&state).to_owned();
            let new_position = playback_position_micros(&state);
            let new_has_track = playback_track_id(&state).is_some();
            let (status_changed, capabilities_changed) = {
                let mut iface = player_ref.get_mut().await;
                let status_changed = iface.playback_status != new_status;
                let capabilities_changed = iface.has_active_track != new_has_track;
                iface.playback_status = new_status;
                iface.position_micros = new_position;
                iface.has_active_track = new_has_track;
                (status_changed, capabilities_changed)
            };
            let iface = player_ref.get().await;
            let emitter = player_ref.signal_emitter();
            if status_changed {
                iface.playback_status_changed(emitter).await?;
            }
            if capabilities_changed {
                iface.can_play_changed(emitter).await?;
                iface.can_pause_changed(emitter).await?;
                iface.can_go_next_changed(emitter).await?;
                iface.can_go_previous_changed(emitter).await?;
            }
        }
        MprisUpdate::NowPlaying(metadata) => {
            let new_metadata = build_mpris_metadata(&metadata);
            {
                let mut iface = player_ref.get_mut().await;
                iface.metadata = new_metadata;
            }
            let iface = player_ref.get().await;
            iface.metadata_changed(player_ref.signal_emitter()).await?;
        }
    }
    Ok(())
}

fn playback_status_text(state: &PlaybackState) -> &'static str {
    match state {
        PlaybackState::Playing { .. } | PlaybackState::Loading { .. } => "Playing",
        PlaybackState::Paused { .. } => "Paused",
        PlaybackState::Stopped => "Stopped",
    }
}

fn playback_position_micros(state: &PlaybackState) -> i64 {
    let position = match state {
        PlaybackState::Playing { position, .. } | PlaybackState::Paused { position, .. } => {
            *position
        }
        PlaybackState::Loading { .. } | PlaybackState::Stopped => return 0,
    };
    i64::try_from(position.as_micros()).unwrap_or(i64::MAX)
}

fn playback_track_id(state: &PlaybackState) -> Option<TrackId> {
    match state {
        PlaybackState::Loading { track_id }
        | PlaybackState::Playing { track_id, .. }
        | PlaybackState::Paused { track_id, .. } => Some(*track_id),
        PlaybackState::Stopped => None,
    }
}

fn build_mpris_metadata(metadata: &NowPlayingMetadata) -> HashMap<String, OwnedValue> {
    let mut map: HashMap<String, OwnedValue> = HashMap::new();

    let track_id_path = metadata
        .track_id
        .map(track_object_path)
        .unwrap_or_else(|| OwnedObjectPath::try_from(TRACK_ID_NONE).unwrap_or_default());
    map.insert(
        "mpris:trackid".to_owned(),
        owned_value(Value::from(track_id_path)),
    );

    if let Some(duration) = metadata.duration {
        let micros = i64::try_from(duration.as_micros()).unwrap_or(i64::MAX);
        map.insert("mpris:length".to_owned(), owned_value(Value::from(micros)));
    }

    if let Some(title) = metadata.title.as_deref().filter(|value| !value.is_empty()) {
        map.insert(
            "xesam:title".to_owned(),
            owned_value(Value::from(title.to_owned())),
        );
    }
    if let Some(artist) = metadata.artist.as_deref().filter(|value| !value.is_empty()) {
        map.insert(
            "xesam:artist".to_owned(),
            owned_value(Value::from(vec![artist.to_owned()])),
        );
    }
    if let Some(album) = metadata.album.as_deref().filter(|value| !value.is_empty()) {
        map.insert(
            "xesam:album".to_owned(),
            owned_value(Value::from(album.to_owned())),
        );
    }
    if let Some(album_artist) = metadata
        .album_artist
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        map.insert(
            "xesam:albumArtist".to_owned(),
            owned_value(Value::from(vec![album_artist.to_owned()])),
        );
    }
    if let Some(genre) = metadata.genre.as_deref().filter(|value| !value.is_empty()) {
        map.insert(
            "xesam:genre".to_owned(),
            owned_value(Value::from(vec![genre.to_owned()])),
        );
    }
    if let Some(track_number) = metadata.track_number {
        let value = i32::try_from(track_number).unwrap_or(i32::MAX);
        map.insert(
            "xesam:trackNumber".to_owned(),
            owned_value(Value::from(value)),
        );
    }
    if let Some(disc_number) = metadata.disc_number {
        let value = i32::try_from(disc_number).unwrap_or(i32::MAX);
        map.insert(
            "xesam:discNumber".to_owned(),
            owned_value(Value::from(value)),
        );
    }

    map
}

fn track_object_path(track_id: TrackId) -> OwnedObjectPath {
    // Wrap the integer track id in a path under our app's reverse-DNS
    // root so clients can disambiguate Sustain's track ids from other
    // players'. Underscores in path segments are valid per the D-Bus
    // spec, so the `open_sustain` segment is fine as-is.
    let path = format!("/io/github/open_sustain/sustain/track/{}", track_id.get());
    OwnedObjectPath::try_from(path)
        .unwrap_or_else(|_| OwnedObjectPath::try_from(TRACK_ID_NONE).unwrap_or_default())
}

fn owned_value(value: Value<'_>) -> OwnedValue {
    // zbus 5's `Value::try_to_owned` is fallible only for unrepresentable
    // variants (file descriptors, etc.); the variants we construct above
    // are all simple containers. Fall back to an empty string on the
    // theoretical error to keep the metadata dict well-typed rather than
    // panicking inside the publish path.
    value
        .try_to_owned()
        .unwrap_or_else(|_| OwnedValue::from(zbus::zvariant::Str::from_static("")))
}

struct RootInterface {
    command_sink: Arc<MprisPlaybackSink>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    async fn raise(&self) {
        self.command_sink.invoke(MprisCommand::Raise);
    }

    async fn quit(&self) {
        self.command_sink.invoke(MprisCommand::Quit);
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> &str {
        IDENTITY
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        DESKTOP_ENTRY
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct PlayerInterface {
    command_sink: Arc<MprisPlaybackSink>,
    playback_status: String,
    metadata: HashMap<String, OwnedValue>,
    position_micros: i64,
    /// Tracks whether the current `PlaybackState` carries an active track,
    /// which is what `CanPlay` / `CanPause` / `CanGoNext` / `CanGoPrevious`
    /// gate on for now. The queue layer always allows next/previous to
    /// fall through to a no-op so the buttons can stay always-enabled in
    /// the UI; we mirror that policy here.
    has_active_track: bool,
}

impl PlayerInterface {
    fn new(command_sink: Arc<MprisPlaybackSink>) -> Self {
        Self {
            command_sink,
            playback_status: "Stopped".to_owned(),
            metadata: HashMap::new(),
            position_micros: 0,
            has_active_track: false,
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    async fn next(&self) {
        self.command_sink.invoke(MprisCommand::Next);
    }

    async fn previous(&self) {
        self.command_sink.invoke(MprisCommand::Previous);
    }

    async fn pause(&self) {
        self.command_sink.invoke(MprisCommand::Pause);
    }

    async fn play_pause(&self) {
        self.command_sink.invoke(MprisCommand::PlayPause);
    }

    async fn stop(&self) {
        self.command_sink.invoke(MprisCommand::Stop);
    }

    async fn play(&self) {
        self.command_sink.invoke(MprisCommand::Play);
    }

    /// `Seek` / `SetPosition` / `OpenUri` are deliberately not wired in
    /// the first MPRIS pass. `CanSeek` is `false` so spec-compliant clients
    /// will not call `Seek` or `SetPosition`. Implement them when the
    /// playback layer exposes a position-seek surface that can be driven
    /// from MPRIS' microsecond offsets without round-trip error.
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        &self.playback_status
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        // zbus serializes the value, so this clone is unavoidable — the
        // dict is small (a handful of entries) and only sent on
        // PropertiesChanged plus explicit Get calls.
        let mut clone = HashMap::with_capacity(self.metadata.len());
        for (key, value) in &self.metadata {
            if let Ok(owned) = value.try_clone() {
                clone.insert(key.clone(), owned);
            }
        }
        clone
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.position_micros
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.has_active_track
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.has_active_track
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.has_active_track
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.has_active_track
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sustain_app_runtime::{PlaybackState, TrackId};

    use super::{
        NowPlayingMetadata, build_mpris_metadata, playback_position_micros, playback_status_text,
    };

    fn track_id(value: i64) -> TrackId {
        TrackId::new(value).expect("positive test track id")
    }

    #[test]
    fn playback_status_text_maps_each_variant() {
        assert_eq!(playback_status_text(&PlaybackState::Stopped), "Stopped");
        assert_eq!(
            playback_status_text(&PlaybackState::Loading {
                track_id: track_id(1)
            }),
            "Playing"
        );
        assert_eq!(
            playback_status_text(&PlaybackState::Playing {
                track_id: track_id(1),
                position: Duration::ZERO,
            }),
            "Playing"
        );
        assert_eq!(
            playback_status_text(&PlaybackState::Paused {
                track_id: track_id(1),
                position: Duration::ZERO,
            }),
            "Paused"
        );
    }

    #[test]
    fn playback_position_micros_handles_each_state() {
        assert_eq!(playback_position_micros(&PlaybackState::Stopped), 0);
        assert_eq!(
            playback_position_micros(&PlaybackState::Loading {
                track_id: track_id(1)
            }),
            0
        );
        assert_eq!(
            playback_position_micros(&PlaybackState::Playing {
                track_id: track_id(1),
                position: Duration::from_micros(123_456_789),
            }),
            123_456_789
        );
        assert_eq!(
            playback_position_micros(&PlaybackState::Paused {
                track_id: track_id(1),
                position: Duration::from_secs(42),
            }),
            42_000_000
        );
    }

    #[test]
    fn build_mpris_metadata_includes_track_id_sentinel_when_no_track() {
        let metadata = NowPlayingMetadata::default();
        let map = build_mpris_metadata(&metadata);

        assert!(map.contains_key("mpris:trackid"));
        assert!(!map.contains_key("xesam:title"));
        assert!(!map.contains_key("mpris:length"));
    }

    #[test]
    fn build_mpris_metadata_emits_xesam_fields_for_set_values() {
        let metadata = NowPlayingMetadata {
            track_id: Some(track_id(7)),
            title: Some("Angel".to_owned()),
            artist: Some("Massive Attack".to_owned()),
            album: Some("Mezzanine".to_owned()),
            album_artist: Some("Massive Attack".to_owned()),
            genre: Some("Trip Hop".to_owned()),
            track_number: Some(1),
            disc_number: Some(1),
            duration: Some(Duration::from_secs(383)),
        };

        let map = build_mpris_metadata(&metadata);

        assert!(map.contains_key("mpris:trackid"));
        assert!(map.contains_key("mpris:length"));
        assert!(map.contains_key("xesam:title"));
        assert!(map.contains_key("xesam:artist"));
        assert!(map.contains_key("xesam:album"));
        assert!(map.contains_key("xesam:albumArtist"));
        assert!(map.contains_key("xesam:genre"));
        assert!(map.contains_key("xesam:trackNumber"));
        assert!(map.contains_key("xesam:discNumber"));
    }

    #[test]
    fn build_mpris_metadata_skips_empty_strings() {
        let metadata = NowPlayingMetadata {
            title: Some(String::new()),
            artist: Some(String::new()),
            ..NowPlayingMetadata::default()
        };

        let map = build_mpris_metadata(&metadata);

        assert!(!map.contains_key("xesam:title"));
        assert!(!map.contains_key("xesam:artist"));
    }
}
