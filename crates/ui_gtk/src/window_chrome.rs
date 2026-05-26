// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::gdk;
use gtk::gdk::prelude::ToplevelExt;
use gtk::prelude::*;

use super::{RESIZE_CORNER_SIZE, RESIZE_EDGE_THICKNESS, WINDOW_SHADOW_MARGIN};

pub(crate) fn install_window_state_chrome(
    window: &gtk::ApplicationWindow,
    window_frame: &gtk::Overlay,
) {
    update_window_state_chrome(window, window_frame);

    let window_frame_for_fullscreen = window_frame.clone();
    window.connect_fullscreened_notify(move |window| {
        update_window_state_chrome(window, &window_frame_for_fullscreen);
    });

    let window_frame_for_maximize = window_frame.clone();
    window.connect_maximized_notify(move |window| {
        update_window_state_chrome(window, &window_frame_for_maximize);
    });
}

fn update_window_state_chrome(window: &gtk::ApplicationWindow, window_frame: &gtk::Overlay) {
    let is_floating = !window.is_fullscreen() && !window.is_maximized();
    let margin = if is_floating {
        window_frame.add_css_class("window-frame");
        WINDOW_SHADOW_MARGIN
    } else {
        window_frame.remove_css_class("window-frame");
        0
    };

    window_frame.set_margin_top(margin);
    window_frame.set_margin_end(margin);
    window_frame.set_margin_bottom(margin);
    window_frame.set_margin_start(margin);
}

pub(crate) fn install_resize_handles(shell: &gtk::Overlay, window: &gtk::ApplicationWindow) {
    for (edge, halign, valign, width, height, cursor) in [
        (
            gdk::SurfaceEdge::North,
            gtk::Align::Fill,
            gtk::Align::Start,
            -1,
            RESIZE_EDGE_THICKNESS,
            "n-resize",
        ),
        (
            gdk::SurfaceEdge::East,
            gtk::Align::End,
            gtk::Align::Fill,
            RESIZE_EDGE_THICKNESS,
            -1,
            "e-resize",
        ),
        (
            gdk::SurfaceEdge::South,
            gtk::Align::Fill,
            gtk::Align::End,
            -1,
            RESIZE_EDGE_THICKNESS,
            "s-resize",
        ),
        (
            gdk::SurfaceEdge::West,
            gtk::Align::Start,
            gtk::Align::Fill,
            RESIZE_EDGE_THICKNESS,
            -1,
            "w-resize",
        ),
        (
            gdk::SurfaceEdge::NorthWest,
            gtk::Align::Start,
            gtk::Align::Start,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "nw-resize",
        ),
        (
            gdk::SurfaceEdge::NorthEast,
            gtk::Align::End,
            gtk::Align::Start,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "ne-resize",
        ),
        (
            gdk::SurfaceEdge::SouthEast,
            gtk::Align::End,
            gtk::Align::End,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "se-resize",
        ),
        (
            gdk::SurfaceEdge::SouthWest,
            gtk::Align::Start,
            gtk::Align::End,
            RESIZE_CORNER_SIZE,
            RESIZE_CORNER_SIZE,
            "sw-resize",
        ),
    ] {
        let handle = resize_handle(edge, window, cursor);
        handle.set_halign(halign);
        handle.set_valign(valign);
        handle.set_size_request(width, height);
        shell.add_overlay(&handle);
        shell.set_measure_overlay(&handle, false);
    }
}

fn resize_handle(
    edge: gdk::SurfaceEdge,
    window: &gtk::ApplicationWindow,
    cursor_name: &str,
) -> gtk::Box {
    let handle = gtk::Box::new(gtk::Orientation::Vertical, 0);
    handle.set_cursor_from_name(Some(cursor_name));

    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let window = window.clone();
    let handle_for_gesture = handle.clone();
    click.connect_pressed(move |click, _n_press, x, y| {
        let Some(surface) = window.surface() else {
            return;
        };
        let Ok(toplevel) = surface.downcast::<gdk::Toplevel>() else {
            return;
        };
        let Some(device) = click.current_event_device() else {
            return;
        };
        let (surface_x, surface_y) = handle_for_gesture
            .compute_point(&window, &gtk::graphene::Point::new(x as f32, y as f32))
            .map(|p| (p.x() as f64, p.y() as f64))
            .unwrap_or((x, y));

        toplevel.begin_resize(
            edge,
            Some(&device),
            click.current_button() as i32,
            surface_x,
            surface_y,
            click.current_event_time(),
        );
    });
    handle.add_controller(click);

    handle
}
