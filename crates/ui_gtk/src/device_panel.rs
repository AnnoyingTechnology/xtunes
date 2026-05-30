// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! The device-sync panel shown in the main content column when a device
//! is selected in the DEVICES sidebar section (issues #23 / #24).
//!
//! Layout: a header whose left side identifies the device (name + mount
//! path) and whose right corner holds the configuration options
//! (on-drive format, per-layout settings); below it the ticked-playlist
//! list fills the remaining height in its own contrasting container; and
//! a fixed bottom bar carries the disk-occupation bar (how much of the
//! device the ticked playlists would occupy) alongside the `Forget
//! device` / `Sync` actions. All mutations go through the command
//! controller; all progress flows through the runtime's notification
//! lane, so the panel never schedules its own timers or pokes the status
//! bar.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use sustain_app_runtime::{
    ApplicationCommand, ConnectedDevice, DeviceLayout, FilesPerFolderCap, PlaylistItem,
};

use crate::SharedRuntime;
use crate::command_controller::SharedCommandController;

/// Recomputes the status-bar track/duration/size summary for the device
/// view. Supplied by the main window, which knows the status bar.
type SummaryRefresh = Rc<dyn Fn()>;

#[derive(Clone)]
pub(crate) struct DeviceSyncPanel {
    root: gtk::Box,
    /// Header + playlist container; fills the height above the bottom bar.
    body: gtk::Box,
    /// Fixed footer: occupation bar + Forget / Sync. Lives outside the
    /// body and is re-filled in place when the selection changes.
    bottom_bar: gtk::Box,
    /// The device currently rendered, so the status-bar summary can be
    /// computed for it while the device view is visible.
    current_device: Rc<RefCell<Option<ConnectedDevice>>>,
    /// Recomputes the status-bar track/duration/size summary. Fired on
    /// show, selection toggle, and forget so the summary tracks the
    /// device's selected content instead of a stale earlier view.
    on_summary_refresh: Rc<RefCell<Option<SummaryRefresh>>>,
    runtime: SharedRuntime,
    command_controller: SharedCommandController,
}

impl DeviceSyncPanel {
    pub(crate) fn new(runtime: SharedRuntime, command_controller: SharedCommandController) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("device-sync-panel");
        root.set_hexpand(true);
        root.set_vexpand(true);

        // The playlist container inside the body fills the leftover height
        // and scrolls its own checklist, so the body itself does not
        // scroll. Children carry their own 18px margins to match the
        // bottom bar's padding.
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_hexpand(true);
        body.set_vexpand(true);
        root.append(&body);

        let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bottom_bar.add_css_class("device-sync-bottom-bar");
        root.append(&bottom_bar);

        Self {
            root,
            body,
            bottom_bar,
            current_device: Rc::new(RefCell::new(None)),
            on_summary_refresh: Rc::new(RefCell::new(None)),
            runtime,
            command_controller,
        }
    }

    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Shared handle to the currently-rendered device, so the status-bar
    /// summary callback can resolve its selected tracks while the device
    /// view is visible.
    pub(crate) fn current_device_cell(&self) -> Rc<RefCell<Option<ConnectedDevice>>> {
        self.current_device.clone()
    }

    /// Install the callback that recomputes the status-bar summary. The
    /// panel fires it whenever the device's selected content changes.
    pub(crate) fn set_summary_refresh(&self, refresh: SummaryRefresh) {
        self.on_summary_refresh.replace(Some(refresh));
    }

    fn fire_summary_refresh(&self) {
        if let Some(refresh) = self.on_summary_refresh.borrow().as_ref() {
            refresh();
        }
    }

    /// Render the panel for `device`, ensuring its saved-config row
    /// exists first so configuration commands have something to update.
    /// Call only once the content stack is already showing the device
    /// page, so the summary refresh resolves to the device's content.
    pub(crate) fn show_device(&self, device: ConnectedDevice) {
        let _ = self.runtime.borrow().ensure_device_config(&device);
        self.current_device.replace(Some(device.clone()));
        self.rebuild(&device);
        self.fire_summary_refresh();
    }

    fn clear(container: &gtk::Box) {
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
    }

    fn rebuild(&self, device: &ConnectedDevice) {
        Self::clear(&self.body);

        let config = self
            .runtime
            .borrow()
            .device_config(&device.id)
            .unwrap_or_else(|| sustain_app_runtime::SyncDevice {
                id: device.id.clone(),
                label: device.label.clone(),
                kind: device.kind,
                layout: DeviceLayout::M3u,
                sub_path: String::new(),
                files_per_folder_cap: FilesPerFolderCap::Unlimited,
                volume_id: device.volume_id.clone(),
            });

        // --- Header: device identity, full width ---
        let header = gtk::Box::new(gtk::Orientation::Vertical, 1);
        header.set_margin_top(18);
        header.set_margin_start(18);
        header.set_margin_end(18);
        let title = gtk::Label::new(Some(&config.label));
        title.add_css_class("device-sync-title");
        title.set_xalign(0.0);
        header.append(&title);
        let mount = gtk::Label::new(Some(&format!("Mounted at {}", device.mount_path.display())));
        mount.add_css_class("dim-label");
        mount.set_xalign(0.0);
        header.append(&mount);
        self.body.append(&header);

        // --- Two columns: playlists (left) | settings (right) ---
        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        columns.set_margin_top(12);
        columns.set_margin_start(18);
        columns.set_margin_end(18);
        columns.set_margin_bottom(18);
        columns.set_vexpand(true);

        let playlists_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        playlists_column.set_hexpand(true);
        playlists_column.set_vexpand(true);
        playlists_column.append(&section_label("Playlists to sync"));
        playlists_column.append(&self.build_playlist_container(device));
        columns.append(&playlists_column);

        let settings_column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        settings_column.set_valign(gtk::Align::Start);
        settings_column.set_size_request(SETTINGS_COLUMN_WIDTH, -1);
        settings_column.append(&section_label("On-drive format"));
        let layout_labels: Vec<&str> = DeviceLayout::ALL.iter().map(|l| l.label()).collect();
        let layout_dropdown = gtk::DropDown::from_strings(&layout_labels);
        layout_dropdown.set_selected(config.layout.as_db() as u32);
        {
            let panel = self.clone();
            let device = device.clone();
            layout_dropdown.connect_selected_notify(move |dropdown| {
                if let Some(layout) = DeviceLayout::ALL.get(dropdown.selected() as usize).copied() {
                    panel.command_controller.dispatch_succeeded(
                        ApplicationCommand::SetDeviceLayout {
                            device_id: device.id.clone(),
                            layout,
                        },
                    );
                    panel.rebuild(&device);
                }
            });
        }
        settings_column.append(&layout_dropdown);
        match config.layout {
            DeviceLayout::FolderPerPlaylist => {
                self.append_folder_cap(&settings_column, device, config.files_per_folder_cap)
            }
            DeviceLayout::Pioneer => self.append_pioneer_readiness(&settings_column, device),
            DeviceLayout::M3u => {}
        }
        columns.append(&settings_column);
        self.body.append(&columns);

        // --- Bottom bar (occupation + actions) ---
        self.refresh_bottom_bar(device);
    }

    fn append_folder_cap(
        &self,
        container: &gtk::Box,
        device: &ConnectedDevice,
        current: FilesPerFolderCap,
    ) {
        container.append(&section_label("Files per folder"));
        let labels: Vec<&str> = FilesPerFolderCap::ALL.iter().map(|c| c.label()).collect();
        let dropdown = gtk::DropDown::from_strings(&labels);
        let selected = FilesPerFolderCap::ALL
            .iter()
            .position(|c| *c == current)
            .unwrap_or(0) as u32;
        dropdown.set_selected(selected);
        {
            let panel = self.clone();
            let device = device.clone();
            dropdown.connect_selected_notify(move |dropdown| {
                if let Some(cap) = FilesPerFolderCap::ALL
                    .get(dropdown.selected() as usize)
                    .copied()
                {
                    panel.command_controller.dispatch_succeeded(
                        ApplicationCommand::SetDeviceFilesPerFolderCap {
                            device_id: device.id.clone(),
                            cap,
                        },
                    );
                }
            });
        }
        container.append(&dropdown);
    }

    fn append_pioneer_readiness(&self, container: &gtk::Box, device: &ConnectedDevice) {
        let readiness = self.runtime.borrow().device_analysis_readiness(&device.id);
        container.append(&section_label("Analysis"));
        let summary = gtk::Label::new(Some(&format!(
            "{} tracks · {} missing BPM · {} key · {} waveform",
            readiness.total,
            readiness.missing_bpm,
            readiness.missing_key,
            readiness.missing_waveform,
        )));
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.add_css_class("dim-label");
        container.append(&summary);

        let analyse =
            gtk::Button::with_label(&format!("Analyse {} missing tracks", readiness.analyzable));
        analyse.set_halign(gtk::Align::Start);
        analyse.set_sensitive(readiness.analyzable > 0);
        {
            let panel = self.clone();
            let device_id = device.id.clone();
            analyse.connect_clicked(move |_| {
                panel.command_controller.dispatch_succeeded(
                    ApplicationCommand::AnalyzeDeviceTracks {
                        device_id: device_id.clone(),
                    },
                );
            });
        }
        container.append(&analyse);
    }

    fn build_playlist_container(&self, device: &ConnectedDevice) -> gtk::Box {
        // The enclosing column owns the outer spacing; this is just the
        // contrasting card that fills the column height.
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("device-sync-playlist-container");
        container.set_hexpand(true);
        container.set_vexpand(true);

        let runtime = self.runtime.borrow();
        let selected: std::collections::HashSet<PlaylistItem> =
            runtime.device_selection(&device.id).into_iter().collect();

        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let mut entries: Vec<(PlaylistItem, gtk::CheckButton)> = Vec::new();
        for playlist in runtime.playlists() {
            let item = PlaylistItem::Playlist(playlist.id);
            let check = playlist_check(&playlist.name, PLAYLIST_ICON, selected.contains(&item));
            list.append(&check);
            entries.push((item, check));
        }
        for smart in runtime.smart_playlists() {
            let item = PlaylistItem::SmartPlaylist(smart.id);
            let check = playlist_check(&smart.name, SMART_PLAYLIST_ICON, selected.contains(&item));
            list.append(&check);
            entries.push((item, check));
        }
        drop(runtime);

        if entries.is_empty() {
            let empty = gtk::Label::new(Some("No playlists yet. Create a playlist to sync it."));
            empty.set_xalign(0.0);
            empty.set_valign(gtk::Align::Start);
            empty.add_css_class("dim-label");
            container.append(&empty);
            return container;
        }

        let entries = Rc::new(entries);
        for (_, check) in entries.iter() {
            let panel = self.clone();
            let device = device.clone();
            let entries = entries.clone();
            check.connect_toggled(move |_| {
                let selection: Vec<PlaylistItem> = entries
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .map(|(item, _)| *item)
                    .collect();
                panel.command_controller.dispatch_succeeded(
                    ApplicationCommand::SetDeviceSelection {
                        device_id: device.id.clone(),
                        selection,
                    },
                );
                // Re-evaluate occupation + the Sync action for the new
                // selection, in place — never touching this checklist.
                panel.refresh_bottom_bar(&device);
                // The status-bar summary reflects the device's selected
                // content, so it changes with every toggle too.
                panel.fire_summary_refresh();
            });
        }

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        container.append(&scroller);
        container
    }

    /// (Re)fill the bottom bar from the current plan + device capacity.
    /// Safe to call from a playlist toggle: it only touches the bottom
    /// bar, never the checklist the toggle came from.
    fn refresh_bottom_bar(&self, device: &ConnectedDevice) {
        Self::clear(&self.bottom_bar);

        let runtime = self.runtime.borrow();
        let plan = runtime.device_sync_plan(&device.id);
        let capacity = runtime.mount_capacity(&device.mount_path);
        drop(runtime);

        let selected_bytes = plan.as_ref().map(|p| p.bytes_total).unwrap_or(0);
        let total_bytes = capacity.map(|c| c.total_bytes).unwrap_or(0);
        let available_bytes = capacity.map(|c| c.available_bytes).unwrap_or(0);
        let needed_bytes = plan.as_ref().map(|p| p.bytes_to_copy).unwrap_or(0);
        let over_capacity = total_bytes > 0 && needed_bytes > available_bytes;

        let (fraction, text) = if total_bytes == 0 {
            (0.0, "Device capacity unavailable".to_owned())
        } else if over_capacity {
            (
                1.0,
                format!(
                    "Not enough space — {} needed, {} free",
                    human_bytes(needed_bytes),
                    human_bytes(available_bytes)
                ),
            )
        } else {
            (
                selected_bytes as f64 / total_bytes as f64,
                format!(
                    "{} of {}",
                    human_bytes(selected_bytes),
                    human_bytes(total_bytes)
                ),
            )
        };

        // One total segment today; a per-genre breakdown will pass several
        // coloured segments to the same renderer (issue #23 follow-up).
        let bar = occupation_bar(
            vec![BarSegment {
                fraction,
                color: None,
            }],
            &text,
            over_capacity,
        );
        self.bottom_bar.append(&bar);

        // --- Actions (extreme right) ---
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let forget = gtk::Button::with_label("Forget device");
        {
            let panel = self.clone();
            let device_id = device.id.clone();
            forget.connect_clicked(move |_| {
                panel
                    .command_controller
                    .dispatch_succeeded(ApplicationCommand::ForgetDevice {
                        device_id: device_id.clone(),
                    });
                Self::clear(&panel.body);
                Self::clear(&panel.bottom_bar);
                let note = gtk::Label::new(Some(
                    "Device forgotten. Its saved playlists and sync history were cleared.",
                ));
                note.set_xalign(0.0);
                note.set_valign(gtk::Align::Start);
                note.set_margin_top(18);
                note.set_margin_start(18);
                note.set_margin_end(18);
                panel.body.append(&note);
                // The selection is gone; the summary now reads empty.
                panel.current_device.replace(None);
                panel.fire_summary_refresh();
            });
        }
        actions.append(&forget);

        let sync = gtk::Button::with_label("Sync");
        sync.add_css_class("suggested-action");
        sync.set_sensitive(plan.is_some() && !over_capacity);
        {
            let command_controller = self.command_controller.clone();
            let root = self.root.clone();
            let device_id = device.id.clone();
            let remove_count = plan.as_ref().map(|p| p.to_remove.len()).unwrap_or(0);
            sync.connect_clicked(move |_| {
                let dispatch = {
                    let command_controller = command_controller.clone();
                    let device_id = device_id.clone();
                    move |remove_stale: bool| {
                        command_controller.dispatch_succeeded(ApplicationCommand::SyncDevice {
                            device_id: device_id.clone(),
                            remove_stale,
                        });
                    }
                };
                // Removals are destructive, so confirm before a sync that
                // would delete tracks the device no longer needs.
                if remove_count > 0 {
                    match root.root().and_downcast::<gtk::Window>() {
                        Some(parent) => {
                            confirm_sync_removals(&parent, remove_count, move || dispatch(true));
                        }
                        // No window to host the dialog (should not happen
                        // while visible): sync the additions, keep stale
                        // files rather than deleting without consent.
                        None => dispatch(false),
                    }
                } else {
                    dispatch(true);
                }
            });
        }
        actions.append(&sync);
        self.bottom_bar.append(&actions);
    }
}

fn section_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("device-sync-section");
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label
}

/// Width of the right-hand settings column.
const SETTINGS_COLUMN_WIDTH: i32 = 260;

/// Sidebar-consistent icons for the two playlist kinds in the checklist.
const PLAYLIST_ICON: &str = "view-list-symbolic";
const SMART_PLAYLIST_ICON: &str = "emblem-system-symbolic";

/// A checklist row: a check button whose child is the playlist's icon
/// (normal vs smart, matching the sidebar) followed by its name.
fn playlist_check(name: &str, icon: &str, active: bool) -> gtk::CheckButton {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&gtk::Image::from_icon_name(icon));
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&label);

    let check = gtk::CheckButton::new();
    check.set_child(Some(&row));
    check.set_active(active);
    check
}

/// One coloured run of the occupation bar, as a fraction of the device
/// capacity. `color: None` uses the widget's accent (the single "total"
/// segment shown today); the planned per-genre view passes explicit
/// palette colours.
#[derive(Clone, Copy)]
struct BarSegment {
    fraction: f64,
    color: Option<gdk::RGBA>,
}

/// The disk-occupation meter: a button-height, button-radius pill that
/// paints `segments` left-to-right over a faint trough and centres `text`
/// over them. Trough/fill colours and the rounded clip come from CSS so
/// the meter tracks the theme and matches the adjacent buttons; over
/// capacity it switches to the theme's error colour. Returned as a plain
/// widget so the bottom bar's horizontal box stretches it to the button
/// height.
fn occupation_bar(segments: Vec<BarSegment>, text: &str, over_capacity: bool) -> gtk::Widget {
    let frame = gtk::Overlay::new();
    frame.add_css_class("device-occupation-bar");
    frame.set_hexpand(true);
    // Clip the fill to the CSS border-radius.
    frame.set_overflow(gtk::Overflow::Hidden);

    let fill = gtk::DrawingArea::new();
    fill.add_css_class("device-occupation-fill");
    if over_capacity {
        frame.add_css_class("over-capacity");
        fill.add_css_class("over-capacity");
    }
    // hexpand only: the bar fills the row's height through the default
    // valign=Fill, NOT vexpand — vexpand would propagate up through the
    // Overlay to the bottom bar and make the whole footer claim vertical
    // space, ballooning it (and the buttons) far past the button height.
    fill.set_hexpand(true);
    fill.set_draw_func(move |area, cr, width, height| {
        let w = width as f64;
        let h = height as f64;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let accent = area.color();
        let mut x = 0.0;
        for segment in &segments {
            let segment_width = (segment.fraction.clamp(0.0, 1.0) * w).min(w - x);
            if segment_width <= 0.0 {
                continue;
            }
            let color = segment.color.unwrap_or(accent);
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                0.45,
            );
            cr.rectangle(x, 0.0, segment_width, h);
            let _ = cr.fill();
            x += segment_width;
        }
    });
    frame.set_child(Some(&fill));

    let label = gtk::Label::new(Some(text));
    label.add_css_class("device-occupation-label");
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Fill);
    label.set_xalign(0.5);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_margin_start(10);
    label.set_margin_end(10);
    frame.add_overlay(&label);

    frame.upcast()
}

/// Confirm a sync that would delete `remove_count` stale tracks from the
/// device. Mirrors the trash-confirmation dialog: a small modal with
/// Cancel as the default. `on_confirm` fires only on the destructive
/// button.
fn confirm_sync_removals(
    parent: &gtk::Window,
    remove_count: usize,
    on_confirm: impl Fn() + 'static,
) {
    let window = gtk::Window::builder()
        .title("Sync device")
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .default_width(440)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(18);
    content.set_margin_end(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);

    let detail = gtk::Label::new(Some(&format!(
        "Syncing will remove {} {} from this device that {} no longer in your selected playlists.",
        remove_count,
        if remove_count == 1 { "track" } else { "tracks" },
        if remove_count == 1 { "is" } else { "are" },
    )));
    detail.add_css_class("dim-label");
    detail.set_xalign(0.0);
    detail.set_wrap(true);
    content.append(&detail);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);

    let cancel = gtk::Button::with_label("Cancel");
    let confirm = gtk::Button::with_label("Sync and remove");
    confirm.add_css_class("destructive-action");

    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }
    {
        let window = window.clone();
        confirm.connect_clicked(move |_| {
            on_confirm();
            window.close();
        });
    }

    buttons.append(&cancel);
    buttons.append(&confirm);
    content.append(&buttons);
    window.set_child(Some(&content));
    window.set_default_widget(Some(&cancel));

    let key_controller = gtk::EventControllerKey::new();
    {
        let window = window.clone();
        key_controller.connect_key_pressed(move |_controller, key, _keycode, _state| {
            if key == gdk::Key::Escape {
                window.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    window.add_controller(key_controller);

    window.present();
    cancel.grab_focus();
}

/// Format a byte count in SI units (powers of 1000), matching how the
/// desktop reports disk sizes so "14.9 GB" lines up with the file
/// manager.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
