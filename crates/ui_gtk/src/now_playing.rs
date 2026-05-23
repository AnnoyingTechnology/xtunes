// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use gtk::prelude::*;
use gtk::{cairo, gdk, glib};

use super::{
    APP_ID, NOW_PLAYING_ICON_SIZE, NOW_PLAYING_SIDE_WIDTH, SharedRuntime, TITLEBAR_HEIGHT,
    command_controller::SharedCommandController,
};
use model::{
    artist_album_text, playback_position, progress_fraction, progress_fraction_from_x,
    remaining_time_text, time_text, track_title,
};
use sustain_app_runtime::{ApplicationCommand, NowPlaying, PlaybackCommand, Track};

mod model;

#[derive(Clone)]
pub(crate) struct NowPlayingView {
    runtime: SharedRuntime,
    area: gtk::Box,
    stack: gtk::Stack,
    title: MarqueeLabel,
    artist_album: MarqueeLabel,
    elapsed: gtk::Label,
    remaining: gtk::Label,
    progress: gtk::ProgressBar,
    shuffle_icon: gtk::Image,
    shuffle_button: gtk::Button,
    repeat_icon: gtk::Image,
    repeat_button: gtk::Button,
    artwork_image: gtk::Image,
    artwork_path: Rc<RefCell<Option<PathBuf>>>,
    duration: Rc<Cell<Duration>>,
}

#[derive(Clone)]
struct MarqueeLabel {
    root: gtk::Overlay,
    canvas: gtk::DrawingArea,
    draw_model: MarqueeDrawModel,
    x_position: Rc<Cell<f64>>,
    paused: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct MarqueeDrawModel {
    text: Rc<RefCell<String>>,
    text_width: Rc<Cell<f64>>,
    x_position: Rc<Cell<f64>>,
    fade_active: Rc<Cell<bool>>,
    style: MarqueeTextStyle,
}

struct SideStatusControl {
    widget: gtk::Box,
    button: gtk::Button,
    icon: gtk::Image,
}

const EMPTY_STACK_NAME: &str = "no-track";
const LOADED_STACK_NAME: &str = "loaded";
const EMPTY_STATE_ICON_SIZE: i32 = 48;
const MARQUEE_EDGE_FADE_WIDTH: f64 = 28.0;
const MARQUEE_FRAME_MS: u64 = 33;
const MARQUEE_HEIGHT: i32 = 19;
const MARQUEE_LOOP_GAP: f64 = 48.0;
const MARQUEE_SPEED: f64 = 0.75;
const MARQUEE_VIEWPORT_WIDTH: i32 = 400;

impl NowPlayingView {
    pub(crate) fn new(runtime: SharedRuntime, command_controller: SharedCommandController) -> Self {
        let area = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        area.add_css_class("now-playing-area");
        area.set_size_request(super::NOW_PLAYING_WIDTH, TITLEBAR_HEIGHT);
        area.set_hexpand(false);
        area.set_halign(gtk::Align::Center);
        area.set_margin_start(super::NOW_PLAYING_HORIZONTAL_MARGIN);
        area.set_margin_end(super::NOW_PLAYING_HORIZONTAL_MARGIN);
        area.set_valign(gtk::Align::Fill);

        let artwork = gtk::Box::new(gtk::Orientation::Vertical, 0);
        artwork.add_css_class("now-playing-artwork");
        artwork.set_size_request(TITLEBAR_HEIGHT, TITLEBAR_HEIGHT);
        artwork.set_overflow(gtk::Overflow::Hidden);

        let artwork_image = gtk::Image::new();
        artwork_image.set_pixel_size(TITLEBAR_HEIGHT);
        artwork_image.set_halign(gtk::Align::Fill);
        artwork_image.set_valign(gtk::Align::Fill);
        artwork_image.set_visible(false);
        artwork.append(&artwork_image);
        let artwork_path: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));

        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.set_hexpand(true);
        details.set_vexpand(true);

        let marquee_paused = Rc::new(Cell::new(false));
        let title = MarqueeLabel::new("now-playing-title", marquee_paused.clone());
        let artist_album = MarqueeLabel::new("now-playing-artist", marquee_paused.clone());
        let metadata = metadata_box(&title, &artist_album);

        let elapsed = time_label();
        let remaining = time_label();
        let shuffle = side_status("media-playlist-shuffle-symbolic", "Shuffle", &elapsed);
        let repeat = side_status("media-playlist-repeat-symbolic", "Repeat", &remaining);
        let detail_content = gtk::CenterBox::new();
        detail_content.set_hexpand(true);
        detail_content.set_vexpand(true);
        detail_content.set_valign(gtk::Align::Fill);
        detail_content.set_start_widget(Some(&shuffle.widget));
        detail_content.set_center_widget(Some(&metadata));
        detail_content.set_end_widget(Some(&repeat.widget));

        let progress = gtk::ProgressBar::new();
        progress.add_css_class("song-progress");
        progress.set_fraction(0.0);
        progress.set_hexpand(true);
        progress.set_halign(gtk::Align::Fill);
        progress.set_valign(gtk::Align::End);
        progress.set_cursor_from_name(Some("pointer"));

        let duration = Rc::new(Cell::new(Duration::ZERO));
        install_progress_seeking(&progress, command_controller.clone(), duration.clone());

        details.append(&detail_content);
        details.append(&progress);

        let loaded_view = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        loaded_view.set_hexpand(true);
        loaded_view.set_vexpand(true);
        loaded_view.append(&artwork);
        loaded_view.append(&details);

        let empty_view = empty_state_view();

        let stack = gtk::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        stack.set_hhomogeneous(true);
        stack.set_vhomogeneous(true);
        stack.add_named(&empty_view, Some(EMPTY_STACK_NAME));
        stack.add_named(&loaded_view, Some(LOADED_STACK_NAME));
        stack.set_visible_child_name(EMPTY_STACK_NAME);
        area.append(&stack);

        install_hover_pause(&area, &title, &artist_album, marquee_paused);

        let view = Self {
            runtime: runtime.clone(),
            area,
            stack,
            title,
            artist_album,
            elapsed,
            remaining,
            progress,
            shuffle_icon: shuffle.icon,
            shuffle_button: shuffle.button,
            repeat_icon: repeat.icon,
            repeat_button: repeat.button,
            artwork_image,
            artwork_path,
            duration,
        };
        install_playback_option_controls(&view, command_controller);
        view.refresh(&runtime.borrow().now_playing());
        install_refresh_timer(&view, runtime);
        view
    }

    pub(crate) fn widget(&self) -> gtk::Box {
        self.area.clone()
    }

    fn sync_artwork(&self, track: Option<&Track>) {
        let new_path = track.and_then(|track| self.runtime.borrow().absolute_track_path(track));
        {
            let current = self.artwork_path.borrow();
            if *current == new_path {
                return;
            }
        }
        *self.artwork_path.borrow_mut() = new_path.clone();

        let texture = new_path
            .as_deref()
            .and_then(|path| self.runtime.borrow().read_artwork(path))
            .and_then(texture_from_bytes);

        match texture {
            Some(texture) => {
                self.artwork_image.set_paintable(Some(&texture));
                self.artwork_image.set_visible(true);
            }
            None => {
                self.artwork_image.set_paintable(None::<&gdk::Paintable>);
                self.artwork_image.set_visible(false);
            }
        }
    }

    pub(crate) fn refresh(&self, now_playing: &NowPlaying) {
        self.sync_artwork(now_playing.track.as_ref());

        let Some(track) = &now_playing.track else {
            self.stack.set_visible_child_name(EMPTY_STACK_NAME);
            self.title.set_text("");
            self.artist_album.set_text("");
            self.elapsed.set_text("");
            self.remaining.set_text("");
            self.progress.set_fraction(0.0);
            self.duration.set(Duration::ZERO);
            sync_playback_option_icon(&self.shuffle_icon, now_playing.options.shuffle_enabled);
            sync_playback_option_icon(&self.repeat_icon, now_playing.options.repeat_enabled());
            return;
        };

        self.stack.set_visible_child_name(LOADED_STACK_NAME);

        let duration = track.metadata.duration.unwrap_or_default();
        self.duration.set(duration);
        let position = playback_position(&now_playing.state).unwrap_or_default();
        self.title.set_text(&track_title(track));
        self.artist_album
            .set_text(&artist_album_text(&track.metadata));
        self.elapsed.set_text(&time_text(position));
        self.remaining
            .set_text(&remaining_time_text(position, duration));
        self.progress
            .set_fraction(progress_fraction(position, duration));
        sync_playback_option_icon(&self.shuffle_icon, now_playing.options.shuffle_enabled);
        sync_playback_option_icon(&self.repeat_icon, now_playing.options.repeat_enabled());
    }
}

fn texture_from_bytes(bytes: Vec<u8>) -> Option<gdk::Texture> {
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes)).ok()?;
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

fn install_playback_option_controls(
    view: &NowPlayingView,
    command_controller: SharedCommandController,
) {
    let command_controller_for_shuffle = command_controller.clone();
    let view_for_shuffle = view.clone();
    view.shuffle_button.connect_clicked(move |_| {
        if command_controller_for_shuffle
            .dispatch_succeeded(ApplicationCommand::Playback(PlaybackCommand::ToggleShuffle))
        {
            view_for_shuffle.refresh(
                &command_controller_for_shuffle
                    .runtime()
                    .borrow()
                    .now_playing(),
            );
        }
    });

    let command_controller_for_repeat = command_controller;
    let view_for_repeat = view.clone();
    view.repeat_button.connect_clicked(move |_| {
        if command_controller_for_repeat
            .dispatch_succeeded(ApplicationCommand::Playback(PlaybackCommand::ToggleRepeat))
        {
            view_for_repeat.refresh(
                &command_controller_for_repeat
                    .runtime()
                    .borrow()
                    .now_playing(),
            );
        }
    });
}

fn install_progress_seeking(
    progress: &gtk::ProgressBar,
    command_controller: SharedCommandController,
    duration: Rc<Cell<Duration>>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.set_exclusive(true);
    let progress_for_click = progress.clone();
    let command_controller_for_click = command_controller.clone();
    let duration_for_click = duration.clone();
    click.connect_pressed(move |click, _press_count, x, _y| {
        claim_gesture_sequence(click);
        commit_seek_from_progress_x(
            &progress_for_click,
            &command_controller_for_click,
            &duration_for_click,
            x,
        );
    });
    progress.add_controller(click);

    let drag = gtk::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    drag.set_exclusive(true);
    let progress_for_drag = progress.clone();
    let duration_for_drag = duration.clone();
    drag.connect_drag_begin(move |drag, x, _y| {
        claim_gesture_sequence(drag);
        preview_progress_from_x(&progress_for_drag, &duration_for_drag, x);
    });

    let progress_for_drag_update = progress.clone();
    let duration_for_drag_update = duration.clone();
    drag.connect_drag_update(move |drag, offset_x, _offset_y| {
        claim_gesture_sequence(drag);
        let Some((start_x, _start_y)) = drag.start_point() else {
            return;
        };
        preview_progress_from_x(
            &progress_for_drag_update,
            &duration_for_drag_update,
            start_x + offset_x,
        );
    });

    let progress_for_drag_end = progress.clone();
    let command_controller_for_drag_end = command_controller.clone();
    let duration_for_drag_end = duration.clone();
    drag.connect_drag_end(move |drag, offset_x, _offset_y| {
        claim_gesture_sequence(drag);
        let Some((start_x, _start_y)) = drag.start_point() else {
            return;
        };
        commit_seek_from_progress_x(
            &progress_for_drag_end,
            &command_controller_for_drag_end,
            &duration_for_drag_end,
            start_x + offset_x,
        );
    });
    progress.add_controller(drag);
}

fn claim_gesture_sequence(gesture: &impl IsA<gtk::Gesture>) {
    let _claimed = gesture.set_state(gtk::EventSequenceState::Claimed);
}

fn preview_progress_from_x(
    progress: &gtk::ProgressBar,
    duration: &Cell<Duration>,
    x: f64,
) -> Option<f64> {
    if duration.get().is_zero() {
        return None;
    }

    let fraction = progress_fraction_from_x(x, progress.width())?;
    progress.set_fraction(fraction);
    Some(fraction)
}

fn commit_seek_from_progress_x(
    progress: &gtk::ProgressBar,
    command_controller: &SharedCommandController,
    duration: &Cell<Duration>,
    x: f64,
) {
    let Some(fraction) = preview_progress_from_x(progress, duration, x) else {
        return;
    };

    commit_seek_to_fraction(command_controller, duration.get(), fraction);
}

fn commit_seek_to_fraction(
    command_controller: &SharedCommandController,
    duration: Duration,
    fraction: f64,
) {
    if duration.is_zero() {
        return;
    }

    let position = duration.mul_f64(fraction.clamp(0.0, 1.0));
    let _result = command_controller.dispatch(ApplicationCommand::Playback(PlaybackCommand::Seek(
        position,
    )));
}

fn install_refresh_timer(view: &NowPlayingView, runtime: SharedRuntime) {
    let view = view.clone();
    glib::timeout_add_seconds_local(1, move || {
        view.refresh(&runtime.borrow().now_playing());
        glib::ControlFlow::Continue
    });
}

fn metadata_box(title: &MarqueeLabel, artist_album: &MarqueeLabel) -> gtk::Box {
    let metadata = gtk::Box::new(gtk::Orientation::Vertical, 0);
    metadata.set_halign(gtk::Align::Center);
    metadata.set_valign(gtk::Align::Center);
    metadata.set_hexpand(true);
    metadata.append(&title.widget());
    metadata.append(&artist_album.widget());
    metadata
}

fn empty_state_view() -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);

    let icon = gtk::Image::from_icon_name(APP_ID);
    icon.add_css_class("now-playing-empty-icon");
    icon.set_pixel_size(EMPTY_STATE_ICON_SIZE);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_hexpand(true);
    icon.set_vexpand(true);
    container.append(&icon);
    container
}

impl MarqueeLabel {
    fn new(css_class: &str, paused: Rc<Cell<bool>>) -> Self {
        let width = MARQUEE_VIEWPORT_WIDTH;
        let root = gtk::Overlay::new();
        root.add_css_class("marquee-label");
        root.set_size_request(width, MARQUEE_HEIGHT);
        root.set_hexpand(false);
        root.set_halign(gtk::Align::Center);
        root.set_overflow(gtk::Overflow::Hidden);

        let canvas = gtk::DrawingArea::new();
        canvas.add_css_class(css_class);
        canvas.set_content_width(width);
        canvas.set_content_height(MARQUEE_HEIGHT);
        canvas.set_size_request(width, MARQUEE_HEIGHT);
        canvas.set_hexpand(false);
        canvas.set_halign(gtk::Align::Center);
        canvas.set_overflow(gtk::Overflow::Hidden);

        let text = Rc::new(RefCell::new(String::new()));
        let text_width = Rc::new(Cell::new(0.0));
        let x_position = Rc::new(Cell::new(0.0));
        let fade_active = Rc::new(Cell::new(false));
        let draw_model = MarqueeDrawModel {
            text,
            text_width,
            x_position: x_position.clone(),
            fade_active,
            style: MarqueeTextStyle::from_css_class(css_class),
        };
        install_marquee_draw_func(&canvas, &draw_model);

        root.set_child(Some(&canvas));

        let marquee = Self {
            root,
            canvas,
            draw_model,
            x_position,
            paused,
        };
        marquee.install_animation();
        marquee
    }

    fn widget(&self) -> gtk::Overlay {
        self.root.clone()
    }

    fn set_text(&self, text: &str) {
        if self.draw_model.text.borrow().as_str() == text {
            return;
        }

        self.draw_model.text.replace(text.to_owned());
        self.reset_to_start();
        self.canvas.queue_draw();
    }

    fn reset_to_start(&self) {
        self.x_position.set(0.0);
        self.canvas.queue_draw();
    }

    fn install_animation(&self) {
        let marquee = self.clone();
        glib::timeout_add_local(Duration::from_millis(MARQUEE_FRAME_MS), move || {
            marquee.advance();
            glib::ControlFlow::Continue
        });
    }

    fn advance(&self) {
        let viewport_width = self.canvas.width();
        let text_width = self.draw_model.text_width.get();
        let overflows = viewport_width > 0 && text_width > f64::from(viewport_width) + 1.0;
        let should_scroll = overflows && !self.paused.get();

        self.draw_model.fade_active.set(should_scroll);

        if !should_scroll {
            self.reset_to_start();
            return;
        }

        let mut x_position = self.x_position.get() - MARQUEE_SPEED;
        if x_position <= -text_width - MARQUEE_LOOP_GAP {
            x_position = 0.0;
        }

        self.x_position.set(x_position);
        self.canvas.queue_draw();
    }
}

fn install_marquee_draw_func(canvas: &gtk::DrawingArea, draw_model: &MarqueeDrawModel) {
    let draw_model = draw_model.clone();

    canvas.set_draw_func(move |canvas, context, width, height| {
        draw_marquee_text(canvas, context, width, height, &draw_model);
    });
}

#[derive(Clone, Copy)]
enum MarqueeTextStyle {
    Title,
    Secondary,
}

impl MarqueeTextStyle {
    fn from_css_class(css_class: &str) -> Self {
        if css_class == "now-playing-title" {
            Self::Title
        } else {
            Self::Secondary
        }
    }

    fn font_size(self) -> f64 {
        match self {
            Self::Title => 14.0,
            Self::Secondary => 12.0,
        }
    }

    fn font_weight(self) -> cairo::FontWeight {
        match self {
            Self::Title => cairo::FontWeight::Bold,
            Self::Secondary => cairo::FontWeight::Normal,
        }
    }

    fn alpha(self) -> f64 {
        match self {
            Self::Title => 1.0,
            Self::Secondary => 0.58,
        }
    }
}

fn draw_marquee_text(
    canvas: &gtk::DrawingArea,
    context: &cairo::Context,
    width: i32,
    height: i32,
    draw_model: &MarqueeDrawModel,
) {
    let text = draw_model.text.borrow();
    if text.is_empty() {
        draw_model.text_width.set(0.0);
        return;
    }

    let _result = context.save();
    context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
    context.clip();
    context.select_font_face(
        "Sans",
        cairo::FontSlant::Normal,
        draw_model.style.font_weight(),
    );
    context.set_font_size(draw_model.style.font_size());
    set_text_source(
        context,
        &canvas.color(),
        draw_model.style.alpha(),
        f64::from(width),
        draw_model.fade_active.get(),
    );

    let Ok(extents) = context.text_extents(&text) else {
        let _result = context.restore();
        return;
    };
    let measured_width = extents.x_advance().max(0.0);
    draw_model.text_width.set(measured_width);

    let x = if measured_width > f64::from(width) + 1.0 {
        draw_model.x_position.get()
    } else {
        (f64::from(width) - measured_width) / 2.0
    };
    let y = (f64::from(height) - extents.height()) / 2.0 - extents.y_bearing();
    draw_text_at(context, &text, x, y);

    if measured_width > f64::from(width) + 1.0 {
        draw_text_at(context, &text, x + measured_width + MARQUEE_LOOP_GAP, y);
    }

    let _result = context.restore();
}

fn set_context_color(context: &cairo::Context, color: &gtk::gdk::RGBA, alpha: f64) {
    context.set_source_rgba(
        f64::from(color.red()),
        f64::from(color.green()),
        f64::from(color.blue()),
        f64::from(color.alpha()) * alpha,
    );
}

fn set_text_source(
    context: &cairo::Context,
    color: &gtk::gdk::RGBA,
    alpha: f64,
    width: f64,
    fade_active: bool,
) {
    if !fade_active || width <= 0.0 {
        set_context_color(context, color, alpha);
        return;
    }

    let gradient = cairo::LinearGradient::new(0.0, 0.0, width, 0.0);
    let red = f64::from(color.red());
    let green = f64::from(color.green());
    let blue = f64::from(color.blue());
    let alpha = f64::from(color.alpha()) * alpha;
    let fade_stop = (MARQUEE_EDGE_FADE_WIDTH / width).clamp(0.0, 0.5);

    gradient.add_color_stop_rgba(0.0, red, green, blue, 0.0);
    gradient.add_color_stop_rgba(fade_stop, red, green, blue, alpha);
    gradient.add_color_stop_rgba(1.0 - fade_stop, red, green, blue, alpha);
    gradient.add_color_stop_rgba(1.0, red, green, blue, 0.0);
    let _result = context.set_source(&gradient);
}

fn draw_text_at(context: &cairo::Context, text: &str, x: f64, y: f64) {
    context.move_to(x, y);
    let _result = context.show_text(text);
}

fn install_hover_pause(
    area: &gtk::Box,
    title: &MarqueeLabel,
    artist_album: &MarqueeLabel,
    marquee_paused: Rc<Cell<bool>>,
) {
    let motion = gtk::EventControllerMotion::new();
    let title_for_enter = title.clone();
    let artist_album_for_enter = artist_album.clone();
    let marquee_paused_for_enter = marquee_paused.clone();
    motion.connect_enter(move |_motion, _x, _y| {
        marquee_paused_for_enter.set(true);
        title_for_enter.reset_to_start();
        artist_album_for_enter.reset_to_start();
    });

    motion.connect_leave(move |_motion| {
        marquee_paused.set(false);
    });
    area.add_controller(motion);
}

fn side_status(icon_name: &str, tooltip: &str, time: &gtk::Label) -> SideStatusControl {
    let status = gtk::Box::new(gtk::Orientation::Vertical, 2);
    status.set_width_request(NOW_PLAYING_SIDE_WIDTH);
    status.set_halign(gtk::Align::Center);
    status.set_valign(gtk::Align::Center);

    let button = gtk::Button::new();
    button.add_css_class("now-playing-side-button");
    button.set_tooltip_text(Some(tooltip));
    button.set_halign(gtk::Align::Center);
    button.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.add_css_class("now-playing-side-icon");
    icon.set_pixel_size(NOW_PLAYING_ICON_SIZE);
    icon.set_halign(gtk::Align::Center);
    button.set_child(Some(&icon));

    status.append(&button);
    status.append(time);

    SideStatusControl {
        widget: status,
        button,
        icon,
    }
}

fn sync_playback_option_icon(icon: &gtk::Image, enabled: bool) {
    if enabled {
        icon.add_css_class("now-playing-side-icon-active");
    } else {
        icon.remove_css_class("now-playing-side-icon-active");
    }
}

fn time_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("now-playing-time");
    label.set_halign(gtk::Align::Center);
    label.set_xalign(0.5);
    label
}
