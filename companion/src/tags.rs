//! Tag and audio-property extraction. Works from a path (CLI) or from bytes (browser).

use std::io::Cursor;
use std::path::Path;

use anyhow::Result;
use lofty::file::{FileType, TaggedFile};
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::Tag;

use crate::format::Codec;
use crate::model::TrackMeta;

pub struct ReadResult {
    pub meta: TrackMeta,
    /// Embedded cover, front cover preferred.
    pub picture: Option<Vec<u8>>,
}

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "m4b", "mp4", "aac", "flac", "ogg", "oga", "opus", "wav", "aif", "aiff", "wv", "ape", "mpc",
];

pub fn read_path(path: &Path) -> Result<ReadResult> {
    let tagged = lofty::read_from_path(path)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    Ok(extract(&tagged, &ext))
}

pub fn read_bytes(bytes: &[u8], ext: &str) -> Result<ReadResult> {
    let tagged = Probe::new(Cursor::new(bytes)).guess_file_type()?.read()?;
    Ok(extract(&tagged, &ext.to_ascii_lowercase()))
}

fn leading_year(s: &str) -> u16 {
    let digits: String = s.trim().chars().take(4).collect();
    if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()) {
        digits.parse().unwrap_or(0)
    } else {
        0
    }
}

/// "-6.52 dB" / "+1.2 dB" / "−3 dB" -> f32
pub fn parse_gain(s: &str) -> Option<f32> {
    let t = s.trim().replace('\u{2212}', "-");
    let num: String = t
        .trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace())
        .trim()
        .to_string();
    num.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn truthy(s: &str) -> bool {
    matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

fn codec_for(tagged: &TaggedFile) -> Codec {
    match tagged.file_type() {
        FileType::Mpeg => Codec::Mp3,
        // lofty's generic properties don't expose the MP4 codec; ALAC reports a bit depth, AAC doesn't.
        FileType::Mp4 => {
            if tagged.properties().bit_depth().is_some() {
                Codec::Alac
            } else {
                Codec::Aac
            }
        }
        FileType::Aac => Codec::Aac,
        FileType::Flac => Codec::Flac,
        FileType::Vorbis => Codec::Vorbis,
        FileType::Opus => Codec::Opus,
        FileType::Wav => Codec::Wav,
        FileType::Aiff => Codec::Aiff,
        FileType::WavPack => Codec::WavPack,
        FileType::Ape => Codec::Ape,
        FileType::Mpc => Codec::Musepack,
        _ => Codec::Unknown,
    }
}

fn get(tag: Option<&Tag>, key: &ItemKey) -> Option<String> {
    tag.and_then(|t| t.get_string(key)).map(|s| s.to_string())
}

fn extract(tagged: &TaggedFile, ext: &str) -> ReadResult {
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let codec = codec_for(tagged);

    let year = tag
        .and_then(|t| t.year())
        .map(|y| y.min(u16::MAX as u32) as u16)
        .filter(|&y| y != 0)
        .or_else(|| get(tag, &ItemKey::RecordingDate).map(|s| leading_year(&s)).filter(|&y| y != 0))
        .or_else(|| get(tag, &ItemKey::Year).map(|s| leading_year(&s)).filter(|&y| y != 0))
        .unwrap_or(0);

    let meta = TrackMeta {
        title: tag.and_then(|t| t.title().map(|s| s.into_owned())),
        artist: tag.and_then(|t| t.artist().map(|s| s.into_owned())),
        album: tag.and_then(|t| t.album().map(|s| s.into_owned())),
        album_artist: get(tag, &ItemKey::AlbumArtist),
        genre: tag.and_then(|t| t.genre().map(|s| s.into_owned())),
        composer: get(tag, &ItemKey::Composer),
        track_no: tag.and_then(|t| t.track()).unwrap_or(0).min(u16::MAX as u32) as u16,
        disc_no: tag.and_then(|t| t.disk()).unwrap_or(0).min(u16::MAX as u32) as u16,
        year,
        compilation: get(tag, &ItemKey::FlagCompilation).map(|s| truthy(&s)).unwrap_or(false),
        audiobook: ext == "m4b",
        duration_ms: props.duration().as_millis().min(u32::MAX as u128) as u32,
        sample_rate: props.sample_rate().unwrap_or(0),
        bits: props.bit_depth().unwrap_or(0),
        channels: props.channels().unwrap_or(0),
        bitrate_kbps: props.audio_bitrate().unwrap_or(0).min(u16::MAX as u32) as u16,
        codec,
        rg_track_db: get(tag, &ItemKey::ReplayGainTrackGain).and_then(|s| parse_gain(&s)),
        rg_album_db: get(tag, &ItemKey::ReplayGainAlbumGain).and_then(|s| parse_gain(&s)),
        ..Default::default()
    };

    let picture = tagged
        .tags()
        .iter()
        .flat_map(|t| t.pictures())
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tagged.tags().iter().flat_map(|t| t.pictures()).next())
        .map(|p| p.data().to_vec());

    ReadResult { meta, picture }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains() {
        assert_eq!(parse_gain("-6.52 dB"), Some(-6.52));
        assert_eq!(parse_gain("+1.20 dB"), Some(1.2));
        assert_eq!(parse_gain("\u{2212}3 dB"), Some(-3.0));
        assert_eq!(parse_gain("junk"), None);
    }

    #[test]
    fn years() {
        assert_eq!(leading_year("2007-10-10"), 2007);
        assert_eq!(leading_year("07"), 0);
    }
}
