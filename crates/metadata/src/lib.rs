#![forbid(unsafe_code)]

use std::path::Path;

pub use xtunes_domain::{MetadataChange, Rating, TrackMetadata};

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

pub trait MetadataService {
    fn read_metadata(&self, path: &Path) -> MetadataResult<TrackMetadata>;
    fn write_metadata(&self, path: &Path, change: MetadataChange) -> MetadataResult<()>;
    fn read_rating(&self, path: &Path) -> MetadataResult<Option<Rating>>;
    fn write_rating(&self, path: &Path, rating: Rating) -> MetadataResult<()>;
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{AudioFormat, MetadataError, audio_format_from_path};

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
}
