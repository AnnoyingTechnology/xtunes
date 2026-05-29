// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::super::{
    ApplicationCommand, command_controller::SharedCommandController,
    date_format::format_system_time_short,
};
use super::{HELPER_MAX_WIDTH_CHARS, HELPER_MIN_WIDTH_CHARS};
use sustain_app_runtime::{SmartShuffleEntropy, SmartShuffleIndexMetadata};

pub(super) fn build(
    window: &gtk::Window,
    command_controller: SharedCommandController,
) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(24);
    content.set_margin_end(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);

    content.append(&build_intro_paragraph());

    let intro_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    intro_separator.add_css_class("preference-section-separator");
    content.append(&intro_separator);

    let initial = command_controller.runtime().borrow().settings().playback;

    let status_section = build_index_status_section(window, command_controller.clone());
    content.append(&status_section);

    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("preference-section-separator");
    content.append(&separator);

    let entropy_section = build_entropy_section(command_controller, initial.smart_shuffle_entropy);
    content.append(&entropy_section);

    content.upcast()
}

/// Preamble at the top of the Shuffle preferences tab. The Smart
/// Shuffle feature is unusual enough — and divisive enough, since
/// classical pure-random shuffle is what most users expect from a
/// shuffle button — that it earns a few sentences of context here.
/// Frame the *why* (preserving the mood or thread you are already in,
/// not chasing variety for its own sake), the *what* (each next track
/// chosen as a continuation of the one playing, by a fixed perceptual
/// match — there is no learning), and the *where* (everything runs on
/// your own machine).
fn build_intro_paragraph() -> gtk::Widget {
    let label = gtk::Label::new(Some(
        "Smart Shuffle picks each next track as a continuation of the one playing now \
         — matching its genre, tempo, key, era and the discovery period it belongs to, \
         to follow the mood or flow you are already in instead of jumping at random. \
         It compares tracks with a fixed, transparent musical metric; there is no \
         learning and nothing leaves your computer. Turning on audio analysis (in the \
         Analysis tab) lets it also match loudness and timbre for smoother transitions.",
    ));
    label.add_css_class("preference-helper");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_natural_wrap_mode(gtk::NaturalWrapMode::Word);
    label.set_width_chars(HELPER_MIN_WIDTH_CHARS);
    label.set_max_width_chars(HELPER_MAX_WIDTH_CHARS);
    label.upcast()
}

fn build_index_status_section(
    window: &gtk::Window,
    command_controller: SharedCommandController,
) -> gtk::Widget {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let header = gtk::Label::new(Some("Index"));
    header.set_xalign(0.0);
    container.append(&header);

    // Read-only status. The index rebuilds itself on the events that
    // actually change it (library scan, audio-analysis coverage, app
    // launch), so there is nothing to schedule and no button to press —
    // this section just reports the current state.
    let status_caption = gtk::Label::new(None);
    status_caption.add_css_class("preference-helper");
    status_caption.set_xalign(0.0);
    status_caption.set_wrap(true);
    status_caption.set_natural_wrap_mode(gtk::NaturalWrapMode::Word);
    status_caption.set_width_chars(HELPER_MIN_WIDTH_CHARS);
    status_caption.set_max_width_chars(HELPER_MAX_WIDTH_CHARS);
    container.append(&status_caption);

    // Live state refresh, driven by the runtime's smart-shuffle state
    // observer. Each fire goes through `idle_add_local_once` because
    // the runtime is mid-borrow when its `apply_smart_shuffle_rebuild_result`
    // emits the signal — we must not re-borrow synchronously.
    let runtime_for_refresh = command_controller.runtime();
    let caption_for_refresh = status_caption.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let runtime = runtime_for_refresh.borrow();
        let is_rebuilding = runtime.smart_shuffle_is_rebuilding();
        let metadata = runtime.smart_shuffle_metadata();
        let index_loaded = runtime.smart_shuffle_index_is_loaded();
        caption_for_refresh.set_text(&status_caption_text(is_rebuilding, index_loaded, metadata));
    });
    refresh();

    let runtime_for_install = command_controller.runtime();
    let refresh_for_observer = refresh.clone();
    runtime_for_install
        .borrow_mut()
        .set_smart_shuffle_state_observer(Box::new(move || {
            let refresh = refresh_for_observer.clone();
            glib::idle_add_local_once(move || {
                refresh();
            });
        }));

    // Clear the observer when the Preferences window closes so the
    // closure (and the widgets it captures) can be dropped — without
    // this, the runtime would hold a stale Box<dyn Fn> pointing at
    // dead widgets across the application lifetime.
    let runtime_for_close = command_controller.runtime();
    window.connect_close_request(move |_| {
        runtime_for_close
            .borrow_mut()
            .clear_smart_shuffle_state_observer();
        glib::Propagation::Proceed
    });

    container.upcast()
}

fn build_entropy_section(
    command_controller: SharedCommandController,
    initial_entropy: SmartShuffleEntropy,
) -> gtk::Widget {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 4);
    container.add_css_class("preference-slider-row");

    let header = gtk::Label::new(Some("Exploration"));
    header.set_xalign(0.0);
    container.append(&header);

    // Three discrete stops (Focused / Balanced / Adventurous)
    // controlling both the candidate-pool width and the softmax
    // temperature applied to candidate scores. Same shape as the
    // analysis tab's resource-usage slider: tick marks at 0/1/2,
    // snap-to-tick via `round_digits = 0`, and a separate label row
    // for the mark text so end-cap labels don't overflow the scale
    // bounds.
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 2.0, 1.0);
    scale.set_round_digits(0);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_mark(0.0, gtk::PositionType::Bottom, None);
    scale.add_mark(1.0, gtk::PositionType::Bottom, None);
    scale.add_mark(2.0, gtk::PositionType::Bottom, None);
    scale.set_value(entropy_to_value(initial_entropy));
    container.append(&scale);

    let mark_label_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    mark_label_row.append(&mark_label("Focused", gtk::Align::Start));
    mark_label_row.append(&mark_label("Balanced", gtk::Align::Center));
    mark_label_row.append(&mark_label("Adventurous", gtk::Align::End));
    container.append(&mark_label_row);

    let caption = gtk::Label::new(Some(entropy_caption(initial_entropy)));
    caption.add_css_class("preference-helper");
    caption.set_xalign(0.0);
    caption.set_wrap(true);
    caption.set_natural_wrap_mode(gtk::NaturalWrapMode::Word);
    caption.set_width_chars(HELPER_MIN_WIDTH_CHARS);
    caption.set_max_width_chars(HELPER_MAX_WIDTH_CHARS);
    container.append(&caption);

    let controller = command_controller;
    let caption_for_callback = caption.clone();
    scale.connect_value_changed(move |s| {
        let entropy = value_to_entropy(s.value());
        caption_for_callback.set_text(entropy_caption(entropy));
        let mut settings = controller.runtime().borrow().settings().clone();
        if settings.playback.smart_shuffle_entropy == entropy {
            return;
        }
        settings.playback.smart_shuffle_entropy = entropy;
        let _ = controller.dispatch(ApplicationCommand::UpdateSettings(settings));
    });

    container.upcast()
}

fn mark_label(text: &str, align: gtk::Align) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_halign(align);
    label.set_hexpand(true);
    label
}

fn entropy_to_value(entropy: SmartShuffleEntropy) -> f64 {
    match entropy {
        SmartShuffleEntropy::Focused => 0.0,
        SmartShuffleEntropy::Balanced => 1.0,
        SmartShuffleEntropy::Adventurous => 2.0,
    }
}

fn value_to_entropy(value: f64) -> SmartShuffleEntropy {
    let snapped = value.round() as i32;
    match snapped {
        n if n <= 0 => SmartShuffleEntropy::Focused,
        1 => SmartShuffleEntropy::Balanced,
        _ => SmartShuffleEntropy::Adventurous,
    }
}

fn entropy_caption(entropy: SmartShuffleEntropy) -> &'static str {
    match entropy {
        SmartShuffleEntropy::Focused => {
            "Almost always plays the highest-scoring continuation. \
             Predictable picks, less surprise."
        }
        SmartShuffleEntropy::Balanced => {
            "Favours strong continuations but mixes in the occasional \
             looser match. The default."
        }
        SmartShuffleEntropy::Adventurous => {
            "Casts a wider net and spreads picks more evenly across it. \
             More variety, more deep cuts."
        }
    }
}

/// The status caption beneath the rebuild controls. Reports reality
/// (§12): whether the index is ready, how many tracks it covers, how
/// much of the library has audio analysis, and when it was last
/// rebuilt — never "trained", because nothing is trained.
fn status_caption_text(
    is_rebuilding: bool,
    index_loaded: bool,
    metadata: Option<SmartShuffleIndexMetadata>,
) -> String {
    if is_rebuilding {
        return "Rebuilding the Smart Shuffle index — this usually takes a moment.".to_owned();
    }
    match (index_loaded, metadata) {
        (true, Some(meta)) => {
            let mut lines = vec![
                "Smart Shuffle ready.".to_owned(),
                format!(
                    "Library indexed: {} tracks",
                    group_thousands(meta.indexed_track_count)
                ),
                // Framed as an *optional enhancement*, not a measure of
                // whether Smart Shuffle works: the core scorer runs on
                // tag features (genre, tempo, key, year, …) at 0%
                // coverage. This line reports how much of the library
                // has the heavier audio analysis (loudness, timbre) that
                // sharpens continuity — so "0%" reads as "not enhanced
                // yet", not "relies on nothing".
                format!(
                    "Audio-enhanced coverage: {}%",
                    (meta.analysis_coverage * 100.0).round() as i64
                ),
            ];
            if let Some(when) = format_system_time_short(meta.built_at) {
                lines.push(format!("Last index rebuild: {when}"));
            }
            lines.join("\n")
        }
        _ => "Smart Shuffle hasn't built its index yet — it will the first time you turn \
              Smart Shuffle on, and refresh itself whenever your library or its analysis \
              changes."
            .to_owned(),
    }
}

/// Format an integer with thousands separators (e.g. `10243` →
/// `10,243`) for the indexed-track-count line.
fn group_thousands(value: u32) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (len - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::{SmartShuffleEntropy, entropy_to_value, group_thousands, value_to_entropy};

    #[test]
    fn entropy_round_trips_through_slider_value() {
        for entropy in [
            SmartShuffleEntropy::Focused,
            SmartShuffleEntropy::Balanced,
            SmartShuffleEntropy::Adventurous,
        ] {
            assert_eq!(value_to_entropy(entropy_to_value(entropy)), entropy);
        }
    }

    #[test]
    fn entropy_slider_snaps_between_ticks() {
        assert_eq!(value_to_entropy(0.4), SmartShuffleEntropy::Focused);
        assert_eq!(value_to_entropy(0.6), SmartShuffleEntropy::Balanced);
        assert_eq!(value_to_entropy(1.4), SmartShuffleEntropy::Balanced);
        assert_eq!(value_to_entropy(1.6), SmartShuffleEntropy::Adventurous);
        assert_eq!(value_to_entropy(-1.0), SmartShuffleEntropy::Focused);
        assert_eq!(value_to_entropy(99.0), SmartShuffleEntropy::Adventurous);
    }

    #[test]
    fn thousands_grouping_matches_expectations() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(42), "42");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(10_243), "10,243");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }
}
