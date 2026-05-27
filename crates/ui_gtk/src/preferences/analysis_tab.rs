// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use gtk::glib::Propagation;
use gtk::prelude::*;

use super::super::{ApplicationCommand, command_controller::SharedCommandController};
use super::switch_row::build_switch_row;

pub(super) fn build(command_controller: SharedCommandController) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(24);
    content.set_margin_end(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);

    let initial = command_controller.runtime().borrow().settings().analysis;

    let bpm_row = build_switch_row(
        "BPM detection",
        "Runs in the background on tracks missing a BPM value. \
         Never modifies tracks that already have one.",
        initial.bpm,
    );
    wire_analysis_switch(
        &bpm_row.switch,
        command_controller.clone(),
        AnalysisFlag::Bpm,
    );
    content.append(&bpm_row.container);

    let key_row = build_switch_row(
        "Key detection",
        "Runs in the background on tracks missing a musical key. \
         Never modifies tracks that already have one.",
        initial.key,
    );
    wire_analysis_switch(
        &key_row.switch,
        command_controller.clone(),
        AnalysisFlag::Key,
    );
    content.append(&key_row.container);

    let waveform_row = build_switch_row(
        "Waveform generation",
        "Computes the preview and detail waveforms with color, and the beatgrid, \
         on tracks that are missing them. Never modifies tracks that already have them.",
        initial.waveform,
    );
    wire_analysis_switch(
        &waveform_row.switch,
        command_controller,
        AnalysisFlag::Waveform,
    );
    content.append(&waveform_row.container);

    content.upcast()
}

#[derive(Clone, Copy)]
enum AnalysisFlag {
    Bpm,
    Key,
    Waveform,
}

fn wire_analysis_switch(
    switch: &gtk::Switch,
    command_controller: SharedCommandController,
    flag: AnalysisFlag,
) {
    switch.connect_state_set(move |_switch, requested_state| {
        let mut settings = command_controller.runtime().borrow().settings().clone();
        match flag {
            AnalysisFlag::Bpm => settings.analysis.bpm = requested_state,
            AnalysisFlag::Key => settings.analysis.key = requested_state,
            AnalysisFlag::Waveform => settings.analysis.waveform = requested_state,
        }
        if command_controller
            .dispatch(ApplicationCommand::UpdateSettings(settings))
            .is_ok()
        {
            Propagation::Proceed
        } else {
            Propagation::Stop
        }
    });
}
