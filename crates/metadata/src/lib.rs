// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#![forbid(unsafe_code)]

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use lofty::{
    config::WriteOptions,
    picture::{Picture, PictureType},
    prelude::{Accessor, AudioFile, TaggedFileExt},
    tag::{
        ItemKey, Tag,
        items::popularimeter::{Popularimeter, StarRating},
    },
};
use sha2::{Digest, Sha256};

pub use sustain_domain::{
    FieldChange, MetadataChange, Rating, TrackContentHash, TrackMetadata, TrackRelativePath,
};

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
    fn read_artwork(&self, path: &Path) -> MetadataResult<Option<Vec<u8>>>;
    fn write_artwork(&self, path: &Path, artwork: Option<Vec<u8>>) -> MetadataResult<()>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedTrack {
    pub relative_path: TrackRelativePath,
    pub metadata: TrackMetadata,
    pub rating: Rating,
    pub file_size_bytes: Option<u64>,
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
        self.scan_directory(library_path, library_path, &mut scan);
        scan.tracks
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(scan)
    }

    fn scan_directory(&self, library_path: &Path, directory: &Path, scan: &mut LibraryScan) {
        let Ok(entries) = fs::read_dir(directory) else {
            scan.failures.push(ScanFailure {
                path: directory.to_path_buf(),
                error: MetadataError::ReadFailed,
            });
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                scan.failures.push(ScanFailure {
                    path,
                    error: MetadataError::ReadFailed,
                });
                continue;
            };
            if metadata.file_type().is_dir() {
                self.scan_directory(library_path, &path, scan);
            } else if metadata.file_type().is_file() {
                self.scan_file(library_path, path, metadata.len(), scan);
            }
        }
    }

    fn scan_file(
        &self,
        library_path: &Path,
        path: PathBuf,
        file_size_bytes: u64,
        scan: &mut LibraryScan,
    ) {
        if audio_format_from_path(&path).is_err() {
            scan.skipped_unsupported_files += 1;
            return;
        }

        let relative_path = match path
            .strip_prefix(library_path)
            .ok()
            .and_then(|path| TrackRelativePath::new(path.to_path_buf()))
        {
            Some(relative_path) => relative_path,
            None => {
                scan.failures.push(ScanFailure {
                    path,
                    error: MetadataError::ReadFailed,
                });
                return;
            }
        };

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
        scan.tracks.push(ScannedTrack {
            relative_path,
            metadata,
            rating,
            file_size_bytes: Some(file_size_bytes),
        });
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
            grouping: tag
                .and_then(|tag| tag.get_string(ItemKey::ContentGroup))
                .map(ToOwned::to_owned),
            genre: tag.and_then(|tag| tag.genre().map(|value| value.into_owned())),
            track_number: tag.and_then(Accessor::track),
            track_total: tag.and_then(Accessor::track_total),
            disc_number: tag.and_then(Accessor::disk),
            disc_total: tag.and_then(Accessor::disk_total),
            year: tag.and_then(|tag| tag.date().map(|date| i32::from(date.year))),
            compilation: tag
                .and_then(|tag| tag.get_string(ItemKey::FlagCompilation))
                .and_then(parse_flag),
            bpm: tag
                .and_then(|tag| tag.get_string(ItemKey::Bpm))
                .and_then(|value| value.trim().parse::<u32>().ok()),
            key: tag
                .and_then(|tag| tag.get_string(ItemKey::InitialKey))
                .map(ToOwned::to_owned),
            comments: tag.and_then(|tag| tag.comment().map(|value| value.into_owned())),
            lyrics: tag
                .and_then(|tag| tag.get_string(ItemKey::Lyrics))
                .map(ToOwned::to_owned),
            duration: Some(properties.duration()),
            bitrate_kbps: properties.audio_bitrate().or(properties.overall_bitrate()),
            sample_rate_hz: properties.sample_rate(),
            channels: properties.channels(),
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
        apply_text_change(tag, ItemKey::ContentGroup, change.grouping);
        apply_text_change(tag, ItemKey::Genre, change.genre);
        apply_number_change(tag, ItemKey::TrackNumber, change.track_number);
        apply_number_change(tag, ItemKey::TrackTotal, change.track_total);
        apply_number_change(tag, ItemKey::DiscNumber, change.disc_number);
        apply_number_change(tag, ItemKey::DiscTotal, change.disc_total);
        apply_number_change(tag, ItemKey::Year, change.year);
        apply_bool_change(tag, ItemKey::FlagCompilation, change.compilation);
        apply_number_change(tag, ItemKey::Bpm, change.bpm);
        apply_text_change(tag, ItemKey::InitialKey, change.key);
        apply_text_change(tag, ItemKey::Comment, change.comments);
        apply_text_change(tag, ItemKey::Lyrics, change.lyrics);

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

    fn read_artwork(&self, path: &Path) -> MetadataResult<Option<Vec<u8>>> {
        audio_format_from_path(path)?;
        let tagged_file = lofty::read_from_path(path).map_err(|_| MetadataError::ReadFailed)?;
        let Some(tag) = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
        else {
            return Ok(None);
        };

        let picture = tag
            .get_picture_type(PictureType::CoverFront)
            .or_else(|| tag.pictures().first());
        Ok(picture.map(|picture| picture.data().to_vec()))
    }

    fn write_artwork(&self, path: &Path, artwork: Option<Vec<u8>>) -> MetadataResult<()> {
        audio_format_from_path(path)?;
        let mut tagged_file = lofty::read_from_path(path).map_err(|_| MetadataError::ReadFailed)?;
        ensure_primary_tag(&mut tagged_file);
        let tag = tagged_file
            .primary_tag_mut()
            .ok_or(MetadataError::WriteFailed)?;

        // Drop every existing CoverFront picture before writing the new one
        // (or leaving the slot empty). Walk in reverse so the indices stay
        // valid as we remove entries.
        let cover_indices: Vec<usize> = tag
            .pictures()
            .iter()
            .enumerate()
            .filter(|(_, picture)| picture.pic_type() == PictureType::CoverFront)
            .map(|(index, _)| index)
            .collect();
        for index in cover_indices.into_iter().rev() {
            let _removed = tag.remove_picture(index);
        }

        if let Some(bytes) = artwork {
            let mut cursor = std::io::Cursor::new(bytes);
            let mut picture =
                Picture::from_reader(&mut cursor).map_err(|_| MetadataError::WriteFailed)?;
            picture.set_pic_type(PictureType::CoverFront);
            tag.push_picture(picture);
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

pub fn hash_file_content(path: &Path) -> MetadataResult<TrackContentHash> {
    let mut file = fs::File::open(path).map_err(|_| MetadataError::ReadFailed)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|_| MetadataError::ReadFailed)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    TrackContentHash::new(lower_hex(&hasher.finalize())).ok_or(MetadataError::ReadFailed)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
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

fn apply_bool_change(tag: &mut Tag, item_key: ItemKey, change: FieldChange<bool>) {
    match change {
        FieldChange::Unchanged => {}
        FieldChange::Set(value) => {
            tag.insert_text(item_key, if value { "1" } else { "0" }.to_owned());
        }
        FieldChange::Clear => {
            let _removed = tag.take(item_key).count();
        }
    }
}

fn parse_flag(value: &str) -> Option<bool> {
    match value.trim() {
        "1" | "true" | "TRUE" | "True" | "yes" => Some(true),
        "0" | "false" | "FALSE" | "False" | "no" => Some(false),
        _ => None,
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

    use sustain_domain::TrackMetadata;

    use super::{
        AudioFormat, LibraryScanner, MetadataError, MetadataResult, MetadataService, Rating,
        audio_format_from_path, hash_file_content,
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

        let scanned_paths = scan
            .tracks
            .iter()
            .map(|track| track.relative_path.as_path().to_path_buf())
            .collect::<Vec<_>>();
        assert_eq!(
            scanned_paths,
            vec![PathBuf::from("nested/two.flac"), PathBuf::from("one.mp3")]
        );
        assert_eq!(scan.skipped_unsupported_files, 1);
        assert_eq!(scan.failures, Vec::new());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn hash_file_content_returns_sha256_hex() {
        let root = unique_test_directory();
        fs::create_dir_all(&root).expect("create test directory");
        let path = root.join("track.flac");
        fs::write(&path, b"abc").expect("write file");

        let hash = hash_file_content(&path).expect("hash file");

        assert_eq!(
            hash.as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

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

        fn read_artwork(&self, _path: &Path) -> MetadataResult<Option<Vec<u8>>> {
            Ok(None)
        }

        fn write_artwork(&self, _path: &Path, _artwork: Option<Vec<u8>>) -> MetadataResult<()> {
            Ok(())
        }
    }

    fn unique_test_directory() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("sustain_metadata_test_{unique_suffix}"))
    }
}
