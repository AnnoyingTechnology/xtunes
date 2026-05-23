// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;
use sustain_app_runtime::{ApplicationCommand, PlaybackCommand, PlaybackState, VolumePercent};

use super::{
    DEFAULT_VOLUME_PERCENT, MEDIA_ICON_SIZE, NOW_PLAYING_HORIZONTAL_MARGIN,
    PlaybackChangedCallback, TITLEBAR_CONTROL_HEIGHT, TITLEBAR_HEIGHT, TITLEBAR_LEFT_PADDING,
    TITLEBAR_RIGHT_PADDING, VOLUME_MAGNET_THRESHOLD, VOLUME_WIDTH,
    command_controller::SharedCommandController,
};

pub(crate) struct Titlebar {
    pub(crate) widget: gtk::WindowHandle,
    pub(crate) play_pause_icon: gtk::Image,
    previous: gtk::Button,
    play_pause: gtk::Button,
    next: gtk::Button,
    volume: gtk::Scale,
}

pub(crate) fn build_titlebar(now_playing: gtk::Box) -> Titlebar {
    let topbar = gtk::CenterBox::new();
    topbar.add_css_class("titlebar");
    topbar.set_hexpand(true);
    topbar.set_height_request(TITLEBAR_HEIGHT);

    let previous = media_icon_button("media-skip-backward-symbolic", "Previous");
    let play_pause_icon = gtk::Image::from_icon_name("media-playback-start-symbolic");
    play_pause_icon.set_pixel_size(MEDIA_ICON_SIZE);
    let play_pause = media_icon_button_from_image(&play_pause_icon, "Play/Pause");
    let next = media_icon_button("media-skip-forward-symbolic", "Next");
    set_titlebar_control_height(&previous);
    set_titlebar_control_height(&play_pause);
    set_titlebar_control_height(&next);

    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    volume.add_css_class("volume-slider");
    volume.set_value(VolumePercent::from_clamped(DEFAULT_VOLUME_PERCENT).as_scalar());
    volume.set_width_request(VOLUME_WIDTH);
    volume.set_height_request(TITLEBAR_CONTROL_HEIGHT);
    volume.set_draw_value(false);
    volume.set_tooltip_text(Some("Volume"));

    let low_volume_icon = volume_icon("audio-volume-low-symbolic", "Low volume");
    let high_volume_icon = volume_icon("audio-volume-high-symbolic", "High volume");

    let volume_controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    volume_controls.set_valign(gtk::Align::Center);
    volume_controls.append(&low_volume_icon);
    volume_controls.append(&volume);
    volume_controls.append(&high_volume_icon);

    let search = gtk::SearchEntry::new();
    search.add_css_class("topbar-search");
    search.set_placeholder_text(Some("Search"));
    search.set_width_chars(24);
    search.set_valign(gtk::Align::Center);

    let window_controls = gtk::WindowControls::new(gtk::PackType::End);
    window_controls.set_valign(gtk::Align::Center);

    let left_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    left_controls.set_valign(gtk::Align::Center);
    left_controls.set_margin_start(NOW_PLAYING_HORIZONTAL_MARGIN);
    left_controls.set_margin_end(NOW_PLAYING_HORIZONTAL_MARGIN);
    left_controls.append(&previous);
    left_controls.append(&play_pause);
    left_controls.append(&next);
    left_controls.append(&horizontal_spacer(TITLEBAR_LEFT_PADDING / 2));
    left_controls.append(&volume_controls);

    let right_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    right_controls.set_valign(gtk::Align::Center);
    right_controls.append(&search);
    right_controls.append(&horizontal_spacer(TITLEBAR_RIGHT_PADDING));
    right_controls.append(&window_controls);

    topbar.set_start_widget(Some(&left_controls));
    topbar.set_center_widget(Some(&now_playing));
    topbar.set_end_widget(Some(&right_controls));

    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&topbar));
    Titlebar {
        widget: handle,
        previous,
        play_pause,
        play_pause_icon,
        next,
        volume,
    }
}

pub(crate) fn connect_titlebar_playback_controls(
    titlebar: &Titlebar,
    command_controller: SharedCommandController,
    playback_changed: PlaybackChangedCallback,
) {
    let command_controller_for_previous = command_controller.clone();
    let playback_changed_for_previous = playback_changed.clone();
    titlebar.previous.connect_clicked(move |_| {
        if command_controller_for_previous.dispatch_succeeded(ApplicationCommand::Playback(
            PlaybackCommand::PlayPreviousTrack,
        )) {
            playback_changed_for_previous();
        }
    });

    let command_controller_for_play_pause = command_controller.clone();
    let playback_changed_for_play_pause = playback_changed.clone();
    titlebar.play_pause.connect_clicked(move |_| {
        if command_controller_for_play_pause.dispatch_succeeded(ApplicationCommand::Playback(
            PlaybackCommand::TogglePlayPause,
        )) {
            playback_changed_for_play_pause();
        }
    });

    let command_controller_for_next = command_controller.clone();
    let playback_changed_for_next = playback_changed.clone();
    titlebar.next.connect_clicked(move |_| {
        if command_controller_for_next
            .dispatch_succeeded(ApplicationCommand::Playback(PlaybackCommand::PlayNextTrack))
        {
            playback_changed_for_next();
        }
    });

    let volume_syncing = Rc::new(Cell::new(false));
    let command_controller_for_volume = command_controller.clone();
    let volume_syncing_for_change = volume_syncing.clone();
    titlebar.volume.connect_value_changed(move |volume| {
        if volume_syncing_for_change.get() {
            return;
        }

        let value = magnetized_volume_value(volume.value());
        if (volume.value() - value).abs() > f64::EPSILON {
            volume_syncing_for_change.set(true);
            volume.set_value(value);
            volume_syncing_for_change.set(false);
        }

        let _result = command_controller_for_volume.dispatch(ApplicationCommand::Playback(
            PlaybackCommand::SetVolume(VolumePercent::from_scalar(value)),
        ));
    });

    let _result = command_controller.dispatch(ApplicationCommand::Playback(
        PlaybackCommand::SetVolume(VolumePercent::from_clamped(DEFAULT_VOLUME_PERCENT)),
    ));
}

pub(crate) fn sync_play_pause_icon(icon: &gtk::Image, state: &PlaybackState) {
    match state {
        PlaybackState::Playing { .. } => {
            icon.set_icon_name(Some("media-playback-pause-symbolic"));
        }
        PlaybackState::Paused { .. } | PlaybackState::Stopped | PlaybackState::Loading { .. } => {
            icon.set_icon_name(Some("media-playback-start-symbolic"));
        }
    }
}

fn horizontal_spacer(width: i32) -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_width_request(width);
    spacer
}

fn media_icon_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(MEDIA_ICON_SIZE);

    media_icon_button_from_image(&icon, tooltip)
}

fn media_icon_button_from_image(icon: &gtk::Image, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(icon));
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button.add_css_class("media-control");
    set_titlebar_control_height(&button);
    button
}

fn volume_icon(icon_name: &str, tooltip: &str) -> gtk::Image {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.add_css_class("volume-icon");
    icon.set_pixel_size(16);
    icon.set_tooltip_text(Some(tooltip));
    icon
}

fn magnetized_volume_value(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }

    let value = value.clamp(0.0, 1.0);
    if (VOLUME_MAGNET_THRESHOLD..1.0).contains(&value) {
        1.0
    } else {
        value
    }
}

fn set_titlebar_control_height(control: &gtk::Button) {
    control.set_height_request(TITLEBAR_CONTROL_HEIGHT);
}

#[cfg(test)]
mod tests {
    use super::magnetized_volume_value;

    #[test]
    fn volume_values_in_top_guard_range_snap_to_unity() {
        assert_eq!(magnetized_volume_value(0.90), 1.0);
        assert_eq!(magnetized_volume_value(0.95), 1.0);
        assert_eq!(magnetized_volume_value(0.999), 1.0);
    }

    #[test]
    fn volume_values_below_guard_range_are_preserved() {
        assert_eq!(magnetized_volume_value(0.899), 0.899);
    }

    #[test]
    fn volume_values_are_clamped_to_slider_range() {
        assert_eq!(magnetized_volume_value(-1.0), 0.0);
        assert_eq!(magnetized_volume_value(2.0), 1.0);
        assert_eq!(magnetized_volume_value(f64::NAN), 0.0);
    }
}
