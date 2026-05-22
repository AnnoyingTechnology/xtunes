#![forbid(unsafe_code)]

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use main_window::build_main_window;

pub use xtunes_app_runtime::{
    ApplicationCommand, ApplicationQuery, ApplicationRuntime, ApplicationRuntimeError,
    BackgroundTaskStatus, LibraryScanResult, LibraryScanSummary, UserSettings,
    run_library_scan_task,
};

mod accent;
mod albums;
mod app_css;
mod artwork_color;
mod command_controller;
mod content_stack;
mod height_reveal;
mod library_scan;
mod main_window;
mod mode_bar;
mod now_playing;
mod preferences;
mod sidebar;
mod status_bar;
mod titlebar;
mod track_context;
mod track_table;
mod window_chrome;

const TITLEBAR_HEIGHT: i32 = 72;
const TITLEBAR_LEFT_PADDING: i32 = 48;
const TITLEBAR_RIGHT_PADDING: i32 = 0;
const TITLEBAR_CONTROL_HEIGHT: i32 = 42;
const MEDIA_ICON_SIZE: i32 = 32;
const MODE_BAR_HEIGHT: i32 = 34;
const MODE_BUTTON_HEIGHT: i32 = 22;
const NOW_PLAYING_HORIZONTAL_MARGIN: i32 = TITLEBAR_HEIGHT / 2;
const NOW_PLAYING_ICON_SIZE: i32 = 16;
const NOW_PLAYING_SIDE_WIDTH: i32 = 58;
const NOW_PLAYING_WIDTH: i32 = 600;
const PREFERENCES_HEIGHT: i32 = 230;
const PREFERENCES_WIDTH: i32 = 520;
const RESIZE_CORNER_SIZE: i32 = 18;
const RESIZE_EDGE_THICKNESS: i32 = 6;
const SIDEBAR_DEFAULT_WIDTH: i32 = 220;
const SIDEBAR_MIN_WIDTH: i32 = 150;
const SIDEBAR_MAX_WIDTH: i32 = 300;
const STATUS_BAR_HEIGHT: i32 = 28;
const VOLUME_WIDTH: i32 = 192;
const DEFAULT_VOLUME_PERCENT: u8 = 80;
const VOLUME_MAGNET_THRESHOLD: f64 = 0.90;
const WINDOW_SHADOW_MARGIN: i32 = 14;
const APP_ID: &str = "io.github.open_xtunes.xtunes";
const SONGS_VIEW: &str = "songs";
const ALBUMS_VIEW: &str = "albums";
const PLAYLISTS_VIEW: &str = "playlists";

pub(crate) type SharedRuntime = Rc<RefCell<ApplicationRuntime>>;
pub(crate) type LibraryChangedCallback = Rc<dyn Fn()>;
pub(crate) type LibraryChangedHolder = Rc<RefCell<Option<LibraryChangedCallback>>>;
pub(crate) type PlaybackChangedCallback = Rc<dyn Fn()>;

pub fn run(runtime: ApplicationRuntime) {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    let runtime = Rc::new(RefCell::new(runtime));

    app.connect_activate(move |app| {
        let window = build_main_window(app, runtime.clone());
        window.present();
    });

    app.run();
}
