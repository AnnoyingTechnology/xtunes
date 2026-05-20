#![forbid(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use lofty::{
    config::WriteOptions,
    prelude::{Accessor, AudioFile, TaggedFileExt},
    tag::{
        ItemKey, Tag,
        items::popularimeter::{Popularimeter, StarRating},
    },
};

pub use xtunes_domain::{FieldChange, MetadataChange, Rating, TrackMetadata};

pub type MetadataResult<T> = Result<T, MetadataError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioFormat {
    Mp3,
    Ogg,
    Flac,
    Mp4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataError {
    UnsupportedAudioFormat,
    WriteFailed,
    ReadFailed,
}

pub trait MetadataService: Send + Sync {
    fn read_metadata(&self, path: &Path) -> MetadataResult<TrackMetadata>;
    fn write_metadata(&self, path: &Path, change: MetadataChange) -> MetadataResult<()>;
    fn read_rating(&self, path: &Path) -> MetadataResult<Option<Rating>>;
    fn write_rating(&self, path: &Path, rating: Rating) -> MetadataResult<()>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedTrack {
    pub path: PathBuf,
    pub metadata: TrackMetadata,
    pub rating: Rating,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanFailure {
    pub path: PathBuf,
    pub error: MetadataError,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryScan {
    pub tracks: Vec<ScannedTrack>,
    pub skipped_unsupported_files: usize,
    pub failures: Vec<ScanFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LibraryScanError {
    LibraryPathUnavailable,
}

pub struct LibraryScanner<'a, S: ?Sized> {
    metadata_service: &'a S,
}

impl<'a, S> LibraryScanner<'a, S>
where
    S: MetadataService + ?Sized,
{
    pub const fn new(metadata_service: &'a S) -> Self {
        Self { metadata_service }
    }

    pub fn scan(&self, library_path: &Path) -> Result<LibraryScan, LibraryScanError> {
        if !library_path.is_dir() {
            return Err(LibraryScanError::LibraryPathUnavailable);
        }

        let mut scan = LibraryScan::default();
        self.scan_directory(library_path, &mut scan, None::<&dyn Fn(ScannedTrack)>);
        scan.tracks
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(scan)
    }

    /// Scan with a callback invoked after each track is found.
    pub fn scan_incremental<F>(&self, library_path: &Path, on_track: F) -> Result<LibraryScan, LibraryScanError>
    where
        F: Fn(ScannedTrack),
    {
        if !library_path.is_dir() {
            return Err(LibraryScanError::LibraryPathUnavailable);
        }

        let mut scan = LibraryScan::default();
        self.scan_directory(library_path, &mut scan, Some(&on_track));
        scan.tracks
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(scan)
    }

    fn scan_directory<F>(&self, directory: &Path, scan: &mut LibraryScan, on_track: Option<&F>)
    where
        F: Fn(ScannedTrack) + ?Sized,
    {
        let Ok(entries) = fs::read_dir(directory) else {
            scan.failures.push(ScanFailure {
                path: directory.to_path_buf(),
                error: MetadataError::ReadFailed,
            });
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_directory(&path, scan, on_track);
            } else {
                self.scan_file(path, scan, on_track);
            }
        }
    }

    fn scan_file<F>(&self, path: PathBuf, scan: &mut LibraryScan, on_track: Option<&F>)
    where
        F: Fn(ScannedTrack) + ?Sized,
    {
        if audio_format_from_path(&path).is_err() {
            scan.skipped_unsupported_files += 1;
            return;
        }

        let metadata = match self.metadata_service.read_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                scan.failures.push(ScanFailure { path, error });
                return;
            }
        };
        let rating = match self.metadata_service.read_rating(&path) {
            Ok(Some(rating)) => rating,
            Ok(None) => Rating::unrated(),
            Err(error) => {
                scan.failures.push(ScanFailure { path, error });
                return;
            }
        };

        let scanned = ScannedTrack {
            path,
            metadata,
            rating,
        };
        if let Some(cb) = on_track {
            cb(scanned.clone());
        }
        scan.tracks.push(scanned);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoftyMetadataService;

impl MetadataService for LoftyMetadataService {
    fn read_metadata(&self, path: &Path) -> MetadataResult<TrackMetadata> {
        audio_format_from_path(path)?;
        let tagged_file = lofty::read_from_path(path).map_err(|_| MetadataError::ReadFailed)?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());
        let properties = tagged_file.properties();

        Ok(TrackMetadata {
            title: tag.and_then(|tag| tag.title().map(|value| value.into_owned())),
            artist: tag.and_then(|tag| tag.artist().map(|value| value.into_owned())),
            album: tag.and_then(|tag| tag.album().map(|value| value.into_owned())),
            album_artist: tag
                .and_then(|tag| tag.get_string(ItemKey::AlbumArtist))
                .map(ToOwned::to_owned),
            composer: tag
                .and_then(|tag| tag.get_string(ItemKey::Composer))
                .map(ToOwned::to_owned),
            genre: tag.and_then(|tag| tag.genre().map(|value| value.into_owned())),
            track_number: tag.and_then(Accessor::track),
            disc_number: tag.and_then(Accessor::disk),
            year: tag.and_then(|tag| tag.date().map(|date| i32::from(date.year))),
            duration: Some(properties.duration()),
            bitrate_kbps: properties.audio_bitrate().or(properties.overall_bitrate()),
        })
    }

    fn write_metadata(&self, path: &Path, change: MetadataChange) -> MetadataResult<()> {
        audio_format_from_path(path)?;
        let mut tagged_file = lofty::read_from_path(path).map_err(|_| MetadataError::ReadFailed)?;
        ensure_primary_tag(&mut tagged_file);
        let tag = tagged_file
            .primary_tag_mut()
            .ok_or(MetadataError::WriteFailed)?;

        apply_text_change(tag, ItemKey::TrackTitle, change.title);
        apply_text_change(tag, ItemKey::TrackArtist, change.artist);
        apply_text_change(tag, ItemKey::AlbumTitle, change.album);
        apply_text_change(tag, ItemKey::AlbumArtist, change.album_artist);
        apply_text_change(tag, ItemKey::Composer, change.composer);
        apply_text_change(tag, ItemKey::Genre, change.genre);
        apply_number_change(tag, ItemKey::TrackNumber, change.track_number);
        apply_number_change(tag, ItemKey::DiscNumber, change.disc_number);
        apply_number_change(tag, ItemKey::Year, change.year);

        tagged_file
            .save_to_path(path, WriteOptions::default())
            .map_err(|_| MetadataError::WriteFailed)
    }

    fn read_rating(&self, path: &Path) -> MetadataResult<Option<Rating>> {
        audio_format_from_path(path)?;
        let tagged_file = lofty::read_from_path(path).map_err(|_| MetadataError::ReadFailed)?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        Ok(tag
            .and_then(|tag| tag.ratings().next())
            .and_then(|rating| Rating::new(star_rating_value(rating.rating()))))
    }

    fn write_rating(&self, path: &Path, rating: Rating) -> MetadataResult<()> {
        audio_format_from_path(path)?;
        let mut tagged_file = lofty::read_from_path(path).map_err(|_| MetadataError::ReadFailed)?;
        ensure_primary_tag(&mut tagged_file);
        let tag = tagged_file
            .primary_tag_mut()
            .ok_or(MetadataError::WriteFailed)?;

        if rating == Rating::unrated() {
            let _removed = tag.take(ItemKey::Popularimeter).count();
        } else {
            tag.insert_text(
                ItemKey::Popularimeter,
                popularimeter_from_rating(rating).to_string(),
            );
        }

        tagged_file
            .save_to_path(path, WriteOptions::default())
            .map_err(|_| MetadataError::WriteFailed)
    }
}

pub fn audio_format_from_path(path: &Path) -> MetadataResult<AudioFormat> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp3") => Ok(AudioFormat::Mp3),
        Some("ogg") | Some("oga") | Some("opus") => Ok(AudioFormat::Ogg),
        Some("flac") => Ok(AudioFormat::Flac),
        Some("m4a") | Some("m4b") | Some("mp4") => Ok(AudioFormat::Mp4),
        _ => Err(MetadataError::UnsupportedAudioFormat),
    }
}

fn ensure_primary_tag(tagged_file: &mut lofty::file::TaggedFile) {
    if tagged_file.primary_tag().is_some() {
        return;
    }

    tagged_file.insert_tag(Tag::new(tagged_file.primary_tag_type()));
}

fn apply_text_change(tag: &mut Tag, item_key: ItemKey, change: FieldChange<String>) {
    match change {
        FieldChange::Unchanged => {}
        FieldChange::Set(value) => {
            tag.insert_text(item_key, value);
        }
        FieldChange::Clear => {
            let _removed = tag.take(item_key).count();
        }
    }
}

fn apply_number_change<T>(tag: &mut Tag, item_key: ItemKey, change: FieldChange<T>)
where
    T: ToString,
{
    match change {
        FieldChange::Unchanged => {}
        FieldChange::Set(value) => {
            tag.insert_text(item_key, value.to_string());
        }
        FieldChange::Clear => {
            let _removed = tag.take(item_key).count();
        }
    }
}

fn star_rating_value(rating: StarRating) -> u8 {
    match rating {
        StarRating::One => 1,
        StarRating::Two => 2,
        StarRating::Three => 3,
        StarRating::Four => 4,
        StarRating::Five => 5,
    }
}

fn popularimeter_from_rating(rating: Rating) -> Popularimeter<'static> {
    match rating.stars() {
        1 => Popularimeter::musicbee(StarRating::One, 0),
        2 => Popularimeter::musicbee(StarRating::Two, 0),
        3 => Popularimeter::musicbee(StarRating::Three, 0),
        4 => Popularimeter::musicbee(StarRating::Four, 0),
        5 => Popularimeter::musicbee(StarRating::Five, 0),
        _ => unreachable!("unrated ratings are removed before conversion"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
    };

    use xtunes_domain::TrackMetadata;

    use super::{
        AudioFormat, LibraryScanner, MetadataError, MetadataResult, MetadataService, Rating,
        audio_format_from_path,
    };

    #[test]
    fn detects_supported_audio_formats_case_insensitively() {
        assert_eq!(
            audio_format_from_path(Path::new("/music/a.MP3")),
            Ok(AudioFormat::Mp3)
        );
        assert_eq!(
            audio_format_from_path(Path::new("/music/a.ogg")),
            Ok(AudioFormat::Ogg)
        );
        assert_eq!(
            audio_format_from_path(Path::new("/music/a.OPUS")),
            Ok(AudioFormat::Ogg)
        );
        assert_eq!(
            audio_format_from_path(Path::new("/music/a.flac")),
            Ok(AudioFormat::Flac)
        );
        assert_eq!(
            audio_format_from_path(Path::new("/music/a.m4a")),
            Ok(AudioFormat::Mp4)
        );
        assert_eq!(
            audio_format_from_path(Path::new("/music/a.mp4")),
            Ok(AudioFormat::Mp4)
        );
    }

    #[test]
    fn rejects_unsupported_audio_formats() {
        assert_eq!(
            audio_format_from_path(Path::new("/music/a.wav")),
            Err(MetadataError::UnsupportedAudioFormat)
        );
        assert_eq!(
            audio_format_from_path(Path::new("/music/no-extension")),
            Err(MetadataError::UnsupportedAudioFormat)
        );
    }

    #[test]
    fn scanner_recurses_and_ignores_unsupported_files() {
        let root = unique_test_directory();
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("create nested test directory");
        fs::write(root.join("one.mp3"), b"not real audio").expect("write test file");
        fs::write(nested.join("two.flac"), b"not real audio").expect("write test file");
        fs::write(root.join("notes.txt"), b"ignore").expect("write test file");

        let metadata_service =
            FakeMetadataService::for_paths([root.join("one.mp3"), nested.join("two.flac")]);
        let scan = LibraryScanner::new(&metadata_service)
            .scan(&root)
            .expect("scan test directory");

        assert_eq!(scan.tracks.len(), 2);
        assert_eq!(scan.skipped_unsupported_files, 1);
        assert_eq!(scan.failures, Vec::new());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[derive(Default)]
    struct FakeMetadataService {
        tracks: BTreeMap<PathBuf, TrackMetadata>,
    }

    impl FakeMetadataService {
        fn for_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
            Self {
                tracks: paths
                    .into_iter()
                    .map(|path| {
                        (
                            path,
                            TrackMetadata {
                                title: Some("Test".to_owned()),
                                ..TrackMetadata::default()
                            },
                        )
                    })
                    .collect(),
            }
        }
    }

    impl MetadataService for FakeMetadataService {
        fn read_metadata(&self, path: &Path) -> MetadataResult<TrackMetadata> {
            self.tracks
                .get(path)
                .cloned()
                .ok_or(MetadataError::ReadFailed)
        }

        fn write_metadata(
            &self,
            _path: &Path,
            _change: super::MetadataChange,
        ) -> MetadataResult<()> {
            Ok(())
        }

        fn read_rating(&self, _path: &Path) -> MetadataResult<Option<Rating>> {
            Ok(Some(Rating::new(4).expect("valid test rating")))
        }

        fn write_rating(&self, _path: &Path, _rating: Rating) -> MetadataResult<()> {
            Ok(())
        }
    }

    fn unique_test_directory() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("xtunes_metadata_test_{unique_suffix}"))
    }
}
