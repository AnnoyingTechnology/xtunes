use gtk::glib::variant::ToVariant;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering as CmpOrdering;
use std::path::Path;
use std::rc::Rc;
use xtunes_app_runtime::{Track, TrackId};

#[derive(Clone, Debug)]
pub(crate) struct TrackTableRow {
    track_id: Option<TrackId>,
    track_name: String,
    artist: String,
    album: String,
    genre: String,
    year: Option<i32>,
    bpm: Option<u16>,
    bitrate_kbps: Option<u32>,
    file_type: AudioFileType,
    pub(crate) duration_seconds: u64,
    rating: u8,
    plays: u64,
    last_played: Option<String>,
    date_added: String,
    track_number: Option<u32>,
    pub(crate) file_size_bytes: u64,
    is_missing: bool,
}

pub(crate) type TrackActivatedCallback = Rc<dyn Fn(TrackId)>;

struct StatusBinding {
    list_item: gtk::ListItem,
    icon: gtk::Image,
}

type StatusBindingsList = Rc<RefCell<Vec<StatusBinding>>>;

#[derive(Clone)]
pub(crate) struct TrackTable {
    scroller: gtk::ScrolledWindow,
    store: gio::ListStore,
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    status_bindings: StatusBindingsList,
}

impl TrackTable {
    pub(crate) fn widget(&self) -> gtk::ScrolledWindow {
        self.scroller.clone()
    }

    pub(crate) fn replace_rows(&self, rows: Vec<TrackTableRow>) {
        self.store.remove_all();
        for row in rows {
            self.store.append(&glib::BoxedAnyObject::new(row));
        }
    }

    pub(crate) fn set_playing_track_id(&self, playing_track_id: Option<TrackId>) {
        if self.playing_track_id.get() == playing_track_id {
            return;
        }
        self.playing_track_id.set(playing_track_id);
        for binding in self.status_bindings.borrow().iter() {
            refresh_status_icon(&binding.list_item, &binding.icon, playing_track_id);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AudioFileType {
    Flac,
    M4a,
    Mp4,
    Mp3,
    Ogg,
    Unknown,
}

impl AudioFileType {
    fn label(self) -> &'static str {
        match self {
            Self::Flac => "FLAC",
            Self::M4a => "M4A",
            Self::Mp4 => "MP4",
            Self::Mp3 => "MP3",
            Self::Ogg => "OGG",
            Self::Unknown => "",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrackTableColumn {
    TrackName,
    Artist,
    Album,
    Genre,
    Year,
    Bpm,
    Bitrate,
    FileType,
    Duration,
    Rating,
    Plays,
    LastPlayed,
    DateAdded,
    TrackNumber,
}

const TRACK_TABLE_COLUMNS: &[TrackTableColumn] = &[
    TrackTableColumn::TrackName,
    TrackTableColumn::Artist,
    TrackTableColumn::Album,
    TrackTableColumn::Genre,
    TrackTableColumn::Year,
    TrackTableColumn::Bpm,
    TrackTableColumn::Bitrate,
    TrackTableColumn::FileType,
    TrackTableColumn::Duration,
    TrackTableColumn::Rating,
    TrackTableColumn::Plays,
    TrackTableColumn::LastPlayed,
    TrackTableColumn::DateAdded,
    TrackTableColumn::TrackNumber,
];

const EMPTY_STAR: &str = "☆";
const FILLED_STAR: &str = "★";
const MAX_RATING: u8 = 5;
const STATUS_COLUMN_WIDTH: i32 = 26;
const STATUS_ICON_SIZE: i32 = 14;
const STATUS_ICON_PLAYING: &str = "audio-volume-high-symbolic";
const STATUS_ICON_MISSING: &str = "dialog-warning-symbolic";

impl TrackTableRow {
    pub(crate) fn from_track(track: &Track, library_root: Option<&Path>) -> Self {
        let absolute_path =
            library_root.map(|library_root| track.location.absolute_path(library_root));
        let file_metadata = absolute_path
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok());
        let is_missing = track.location.is_missing() || file_metadata.is_none();

        Self {
            track_id: Some(track.id),
            track_name: non_empty_text(&track.metadata.title)
                .or_else(|| file_stem_text(track.location.relative_path.as_path()))
                .unwrap_or_default(),
            artist: non_empty_text(&track.metadata.artist).unwrap_or_default(),
            album: non_empty_text(&track.metadata.album).unwrap_or_default(),
            genre: non_empty_text(&track.metadata.genre).unwrap_or_default(),
            year: track.metadata.year,
            bpm: None,
            bitrate_kbps: track.metadata.bitrate_kbps,
            file_type: AudioFileType::from_path(track.location.relative_path.as_path()),
            duration_seconds: track
                .metadata
                .duration
                .map(|duration| duration.as_secs())
                .unwrap_or_default(),
            rating: track.rating.stars(),
            plays: track.statistics.play_count,
            last_played: None,
            date_added: String::new(),
            track_number: track.metadata.track_number,
            file_size_bytes: file_metadata
                .map(|metadata| metadata.len())
                .unwrap_or_default(),
            is_missing,
        }
    }
}

impl AudioFileType {
    fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("flac") => Self::Flac,
            Some("m4a") | Some("m4b") => Self::M4a,
            Some("mp4") => Self::Mp4,
            Some("mp3") => Self::Mp3,
            Some("ogg") | Some("oga") | Some("opus") => Self::Ogg,
            _ => Self::Unknown,
        }
    }
}

impl TrackTableColumn {
    fn title(self) -> &'static str {
        match self {
            Self::TrackName => "Track Name",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Genre => "Genre",
            Self::Year => "Year",
            Self::Bpm => "BPM",
            Self::Bitrate => "Bitrate",
            Self::FileType => "Type",
            Self::Duration => "Duration",
            Self::Rating => "Rating",
            Self::Plays => "Plays",
            Self::LastPlayed => "Last Played",
            Self::DateAdded => "Date Added",
            Self::TrackNumber => "Track #",
        }
    }

    fn action_name(self) -> &'static str {
        match self {
            Self::TrackName => "track_name",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Genre => "genre",
            Self::Year => "year",
            Self::Bpm => "bpm",
            Self::Bitrate => "bitrate",
            Self::FileType => "file_type",
            Self::Duration => "duration",
            Self::Rating => "rating",
            Self::Plays => "plays",
            Self::LastPlayed => "last_played",
            Self::DateAdded => "date_added",
            Self::TrackNumber => "track_number",
        }
    }

    fn default_width(self) -> i32 {
        match self {
            Self::TrackName => 220,
            Self::Artist => 150,
            Self::Album => 170,
            Self::Genre => 120,
            Self::Year => 72,
            Self::Bpm => 72,
            Self::Bitrate => 90,
            Self::FileType => 72,
            Self::Duration => 86,
            Self::Rating => 94,
            Self::Plays => 76,
            Self::LastPlayed => 120,
            Self::DateAdded => 120,
            Self::TrackNumber => 86,
        }
    }

    fn expands(self) -> bool {
        false
    }

    fn default_visible(self) -> bool {
        true
    }

    fn xalign(self) -> f32 {
        match self {
            Self::TrackName
            | Self::Artist
            | Self::Album
            | Self::Genre
            | Self::FileType
            | Self::LastPlayed
            | Self::DateAdded => 0.0,
            Self::Year
            | Self::Bpm
            | Self::Bitrate
            | Self::Duration
            | Self::Rating
            | Self::Plays
            | Self::TrackNumber => 1.0,
        }
    }

    fn text(self, row: &TrackTableRow) -> String {
        match self {
            Self::TrackName => row.track_name.clone(),
            Self::Artist => row.artist.clone(),
            Self::Album => row.album.clone(),
            Self::Genre => row.genre.clone(),
            Self::Year => optional_number_text(row.year),
            Self::Bpm => optional_number_text(row.bpm),
            Self::Bitrate => row
                .bitrate_kbps
                .map(|bitrate| format!("{bitrate} kbps"))
                .unwrap_or_default(),
            Self::FileType => row.file_type.label().to_owned(),
            Self::Duration => track_duration_text(row.duration_seconds),
            Self::Rating => row.rating.to_string(),
            Self::Plays => row.plays.to_string(),
            Self::LastPlayed => row.last_played.clone().unwrap_or_default(),
            Self::DateAdded => row.date_added.clone(),
            Self::TrackNumber => optional_number_text(row.track_number),
        }
    }

    fn compare(self, left: &TrackTableRow, right: &TrackTableRow) -> CmpOrdering {
        match self {
            Self::TrackName => compare_text(&left.track_name, &right.track_name),
            Self::Artist => compare_text(&left.artist, &right.artist),
            Self::Album => compare_text(&left.album, &right.album),
            Self::Genre => compare_text(&left.genre, &right.genre),
            Self::Year => left.year.cmp(&right.year),
            Self::Bpm => left.bpm.cmp(&right.bpm),
            Self::Bitrate => left.bitrate_kbps.cmp(&right.bitrate_kbps),
            Self::FileType => left.file_type.label().cmp(right.file_type.label()),
            Self::Duration => left.duration_seconds.cmp(&right.duration_seconds),
            Self::Rating => left.rating.cmp(&right.rating),
            Self::Plays => left.plays.cmp(&right.plays),
            Self::LastPlayed => left.last_played.cmp(&right.last_played),
            Self::DateAdded => left.date_added.cmp(&right.date_added),
            Self::TrackNumber => left.track_number.cmp(&right.track_number),
        }
    }
}

pub(crate) fn build_track_table(
    rows: Vec<TrackTableRow>,
    track_activated: Option<TrackActivatedCallback>,
) -> TrackTable {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for row in rows {
        store.append(&glib::BoxedAnyObject::new(row));
    }

    let table = gtk::ColumnView::new(None::<gtk::SelectionModel>);
    table.add_css_class("track-table");
    table.set_hexpand(true);
    table.set_vexpand(true);
    table.set_reorderable(true);
    table.set_show_column_separators(false);
    table.set_show_row_separators(false);
    table.set_single_click_activate(false);

    let playing_track_id: Rc<Cell<Option<TrackId>>> = Rc::new(Cell::new(None));
    let status_bindings: StatusBindingsList = Rc::new(RefCell::new(Vec::new()));

    table.append_column(&build_status_column(
        playing_track_id.clone(),
        status_bindings.clone(),
    ));

    let header_menu = build_column_visibility_menu();
    let column_actions = gio::SimpleActionGroup::new();

    for column in TRACK_TABLE_COLUMNS.iter().copied() {
        let table_column = build_table_column(column, &header_menu);
        let action = gio::SimpleAction::new_stateful(
            column.action_name(),
            None,
            &column.default_visible().to_variant(),
        );
        let table_column_for_action = table_column.clone();
        action.connect_activate(move |action, _parameter| {
            let visible = !table_column_for_action.is_visible();
            table_column_for_action.set_visible(visible);
            action.set_state(&visible.to_variant());
        });
        column_actions.add_action(&action);
        table.append_column(&table_column);
    }
    table.append_column(&build_filler_column());

    table.insert_action_group("columns", Some(&column_actions));

    let sorted_rows = gtk::SortListModel::new(Some(store.clone()), table.sorter());
    let selection = gtk::SingleSelection::new(Some(sorted_rows));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    if let Some(track_activated) = track_activated {
        let selection_for_activate = selection.clone();
        table.connect_activate(move |_table, position| {
            let Some(track_id) = selection_for_activate
                .item(position)
                .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
                .and_then(|row_object| {
                    row_object
                        .try_borrow::<TrackTableRow>()
                        .ok()
                        .and_then(|row| row.track_id)
                })
            else {
                return;
            };

            track_activated(track_id);
        });
    }
    table.set_model(Some(&selection));

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_hexpand(true);
    scroller.set_child(Some(&table));
    TrackTable {
        scroller,
        store,
        playing_track_id,
        status_bindings,
    }
}

fn build_table_column(column: TrackTableColumn, header_menu: &gio::Menu) -> gtk::ColumnViewColumn {
    let factory = if column == TrackTableColumn::Rating {
        build_rating_cell_factory()
    } else {
        build_text_cell_factory(column)
    };
    let table_column = gtk::ColumnViewColumn::new(Some(column.title()), Some(factory));
    table_column.set_resizable(true);
    table_column.set_expand(column.expands());
    table_column.set_fixed_width(column.default_width());
    table_column.set_visible(column.default_visible());
    table_column.set_header_menu(Some(header_menu));

    let sorter =
        gtk::CustomSorter::new(move |left, right| compare_track_objects(column, left, right));
    table_column.set_sorter(Some(&sorter));

    table_column
}

fn build_filler_column() -> gtk::ColumnViewColumn {
    let table_column = gtk::ColumnViewColumn::new(None, Some(build_filler_factory()));
    table_column.set_expand(true);
    table_column.set_resizable(false);
    table_column.set_visible(true);
    table_column
}

fn build_status_column(
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    bindings: StatusBindingsList,
) -> gtk::ColumnViewColumn {
    let factory = build_status_cell_factory(playing_track_id, bindings);
    let table_column = gtk::ColumnViewColumn::new(None, Some(factory));
    table_column.set_resizable(false);
    table_column.set_fixed_width(STATUS_COLUMN_WIDTH);
    table_column.set_visible(true);
    table_column
}

fn build_status_cell_factory(
    playing_track_id: Rc<Cell<Option<TrackId>>>,
    bindings: StatusBindingsList,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();

    let bindings_for_setup = bindings.clone();
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_selection_sync(list_item, &cell);

        let icon = gtk::Image::new();
        icon.set_pixel_size(STATUS_ICON_SIZE);
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        icon.set_hexpand(true);
        icon.add_css_class("track-table-status-icon");
        cell.append(&icon);

        list_item.set_child(Some(&cell));

        bindings_for_setup.borrow_mut().push(StatusBinding {
            list_item: list_item.clone(),
            icon,
        });
    });

    let bindings_for_teardown = bindings;
    factory.connect_teardown(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        bindings_for_teardown
            .borrow_mut()
            .retain(|binding| binding.list_item != *list_item);
    });

    let playing_for_bind = playing_track_id;
    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());

        let Some(icon) = cell
            .first_child()
            .and_then(|child| child.downcast::<gtk::Image>().ok())
        else {
            return;
        };
        refresh_status_icon(list_item, &icon, playing_for_bind.get());
    });

    factory
}

fn refresh_status_icon(
    list_item: &gtk::ListItem,
    icon: &gtk::Image,
    playing_track_id: Option<TrackId>,
) {
    let Some(row_object) = list_item
        .item()
        .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
    else {
        clear_status_icon(icon);
        return;
    };
    let Ok(row) = row_object.try_borrow::<TrackTableRow>() else {
        clear_status_icon(icon);
        return;
    };

    icon.remove_css_class("track-table-status-playing");
    icon.remove_css_class("track-table-status-missing");

    if row.is_missing {
        icon.set_icon_name(Some(STATUS_ICON_MISSING));
        icon.add_css_class("track-table-status-missing");
        icon.set_visible(true);
        return;
    }

    if matches!(
        (row.track_id, playing_track_id),
        (Some(track_id), Some(playing_id)) if track_id == playing_id
    ) {
        icon.set_icon_name(Some(STATUS_ICON_PLAYING));
        icon.add_css_class("track-table-status-playing");
        icon.set_visible(true);
        return;
    }

    clear_status_icon(icon);
}

fn clear_status_icon(icon: &gtk::Image) {
    icon.set_icon_name(None);
    icon.set_visible(false);
}

fn build_text_cell_factory(column: TrackTableColumn) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_selection_sync(list_item, &cell);

        let label = gtk::Label::new(None);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_hexpand(true);
        label.set_valign(gtk::Align::Center);
        label.set_margin_start(8);
        label.set_margin_end(8);
        label.set_xalign(column.xalign());

        cell.append(&label);
        list_item.set_child(Some(&cell));
    });

    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());

        let Some(label) = cell
            .first_child()
            .and_then(|child| child.downcast::<gtk::Label>().ok())
        else {
            return;
        };
        let Some(row_object) = list_item
            .item()
            .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
        else {
            return;
        };
        let Ok(row) = row_object.try_borrow::<TrackTableRow>() else {
            return;
        };

        label.set_text(&column.text(&row));
    });

    factory
}

fn build_rating_cell_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_selection_sync(list_item, &cell);
        list_item.set_child(Some(&cell));
    });

    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());
        clear_box_children(&cell);

        let Some(row_object) = list_item
            .item()
            .and_then(|item| item.downcast::<glib::BoxedAnyObject>().ok())
        else {
            return;
        };
        let Ok(row) = row_object.try_borrow::<TrackTableRow>() else {
            return;
        };
        let rating = row.rating;
        drop(row);

        let rating_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        rating_box.add_css_class("rating-stars");
        rating_box.set_margin_start(6);
        rating_box.set_margin_end(6);
        rating_box.set_halign(gtk::Align::End);
        rating_box.set_valign(gtk::Align::Center);

        for star in 1..=MAX_RATING {
            let button = gtk::Button::with_label("");
            button.add_css_class("flat");
            button.add_css_class("rating-star");
            sync_rating_button(&button, star, rating);

            let row_object_for_click = row_object.clone();
            let rating_box_for_click = rating_box.clone();
            button.connect_clicked(move |_| {
                let Ok(row) = row_object_for_click.try_borrow::<TrackTableRow>() else {
                    return;
                };
                let new_rating = rating_after_click(row.rating, star);
                drop(row);

                let mut row_object = row_object_for_click.clone();
                if let Ok(mut row) = row_object.try_borrow_mut::<TrackTableRow>() {
                    row.rating = new_rating;
                }
                sync_rating_buttons(&rating_box_for_click, new_rating);
            });

            rating_box.append(&button);
        }

        cell.append(&rating_box);
    });

    factory
}

fn build_filler_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };

        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("track-table-cell");
        cell.set_hexpand(true);
        cell.set_vexpand(true);
        cell.set_halign(gtk::Align::Fill);
        cell.set_valign(gtk::Align::Fill);
        install_cell_selection_sync(list_item, &cell);
        list_item.set_child(Some(&cell));
    });

    factory.connect_bind(move |_factory, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item
            .child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        else {
            return;
        };
        apply_row_tint(&cell, list_item.position());
        sync_row_selection_class(&cell, list_item.is_selected());
    });

    factory
}

fn apply_row_tint(cell: &gtk::Box, row_position: u32) {
    cell.remove_css_class("track-table-row-even");
    cell.remove_css_class("track-table-row-odd");
    if row_position % 2 == 0 {
        cell.add_css_class("track-table-row-even");
    } else {
        cell.add_css_class("track-table-row-odd");
    }
}

fn install_cell_selection_sync(list_item: &gtk::ListItem, cell: &gtk::Box) {
    let cell_for_selection = cell.clone();
    list_item.connect_selected_notify(move |list_item| {
        sync_row_selection_class(&cell_for_selection, list_item.is_selected());
    });
    sync_row_selection_class(cell, list_item.is_selected());
}

fn sync_row_selection_class(cell: &gtk::Box, selected: bool) {
    if selected {
        cell.add_css_class("track-table-row-selected");
    } else {
        cell.remove_css_class("track-table-row-selected");
    }
}

fn clear_box_children(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn sync_rating_buttons(rating_box: &gtk::Box, rating: u8) {
    let mut star = 1;
    let mut child = rating_box.first_child();
    while let Some(widget) = child {
        let next_child = widget.next_sibling();
        if let Ok(button) = widget.downcast::<gtk::Button>() {
            sync_rating_button(&button, star, rating);
            star += 1;
        }
        child = next_child;
    }
}

fn sync_rating_button(button: &gtk::Button, star: u8, rating: u8) {
    button.remove_css_class("rating-star-filled");
    button.remove_css_class("rating-star-empty");
    button.set_label(rating_star_label(star, rating));
    if star <= rating {
        button.add_css_class("rating-star-filled");
    } else {
        button.add_css_class("rating-star-empty");
    }
}

fn rating_star_label(star: u8, rating: u8) -> &'static str {
    if star <= rating {
        FILLED_STAR
    } else {
        EMPTY_STAR
    }
}

fn rating_after_click(current_rating: u8, clicked_star: u8) -> u8 {
    let clicked_star = clicked_star.min(MAX_RATING);
    if current_rating == clicked_star {
        0
    } else {
        clicked_star
    }
}

fn build_column_visibility_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let columns = gio::Menu::new();
    for column in TRACK_TABLE_COLUMNS {
        columns.append(
            Some(column.title()),
            Some(&format!("columns.{}", column.action_name())),
        );
    }
    menu.append_section(Some("Columns"), &columns);
    menu
}

fn compare_track_objects(
    column: TrackTableColumn,
    left: &glib::Object,
    right: &glib::Object,
) -> gtk::Ordering {
    let Some(left) = left.downcast_ref::<glib::BoxedAnyObject>() else {
        return gtk::Ordering::Equal;
    };
    let Some(right) = right.downcast_ref::<glib::BoxedAnyObject>() else {
        return gtk::Ordering::Equal;
    };
    let Ok(left) = left.try_borrow::<TrackTableRow>() else {
        return gtk::Ordering::Equal;
    };
    let Ok(right) = right.try_borrow::<TrackTableRow>() else {
        return gtk::Ordering::Equal;
    };

    to_gtk_ordering(column.compare(&left, &right))
}

fn compare_text(left: &str, right: &str) -> CmpOrdering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

fn optional_number_text<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn non_empty_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn file_stem_text(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|file_stem| file_stem.to_str())
        .map(str::trim)
        .filter(|file_stem| !file_stem.is_empty())
        .map(ToOwned::to_owned)
}

fn to_gtk_ordering(ordering: CmpOrdering) -> gtk::Ordering {
    match ordering {
        CmpOrdering::Less => gtk::Ordering::Smaller,
        CmpOrdering::Equal => gtk::Ordering::Equal,
        CmpOrdering::Greater => gtk::Ordering::Larger,
    }
}

fn track_duration_text(duration_seconds: u64) -> String {
    let hours = duration_seconds / 3_600;
    let minutes = duration_seconds % 3_600 / 60;
    let seconds = duration_seconds % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_columns_match_the_product_contract() {
        let titles: Vec<&str> = TRACK_TABLE_COLUMNS
            .iter()
            .map(|column| column.title())
            .collect();

        assert_eq!(
            titles,
            vec![
                "Track Name",
                "Artist",
                "Album",
                "Genre",
                "Year",
                "BPM",
                "Bitrate",
                "Type",
                "Duration",
                "Rating",
                "Plays",
                "Last Played",
                "Date Added",
                "Track #",
            ]
        );
    }

    #[test]
    fn table_columns_have_stable_action_names() {
        let action_names: Vec<&str> = TRACK_TABLE_COLUMNS
            .iter()
            .map(|column| column.action_name())
            .collect();

        assert_eq!(
            action_names,
            vec![
                "track_name",
                "artist",
                "album",
                "genre",
                "year",
                "bpm",
                "bitrate",
                "file_type",
                "duration",
                "rating",
                "plays",
                "last_played",
                "date_added",
                "track_number",
            ]
        );
    }

    #[test]
    fn track_duration_text_uses_minutes_until_an_hour() {
        assert_eq!(track_duration_text(244), "4:04");
    }

    #[test]
    fn track_duration_text_uses_hours_when_needed() {
        assert_eq!(track_duration_text(3_904), "1:05:04");
    }

    #[test]
    fn clicking_a_different_star_sets_that_rating() {
        assert_eq!(rating_after_click(2, 4), 4);
    }

    #[test]
    fn clicking_the_current_rating_clears_rating_to_zero() {
        assert_eq!(rating_after_click(4, 4), 0);
    }

    #[test]
    fn rating_clicks_are_clamped_to_five_stars() {
        assert_eq!(rating_after_click(0, 9), 5);
    }
}
