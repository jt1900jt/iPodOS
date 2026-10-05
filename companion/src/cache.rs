//! Serialised track metadata, so the companion does not re-read every file on a sync
//! that changed nothing.
//!
//! Parsing tags means reading each file, which on a 15 GB library dominates sync time
//! even when nothing has changed. The browser keeps one of these blobs per file, keyed
//! by path, size and mtime, and feeds it back instead of the file's bytes.
//!
//! The blob carries the cover image too: art is rendered from the original JPEG or PNG,
//! so without it a cached track could not contribute artwork.

use anyhow::{bail, Result};

use crate::format::Codec;
use crate::model::TrackMeta;

const MAGIC: [u8; 4] = *b"IPTC";
const VERSION: u16 = 1;

fn put_str(out: &mut Vec<u8>, s: Option<&str>) {
    let b = s.unwrap_or("").as_bytes();
    let n = b.len().min(u16::MAX as usize);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&b[..n]);
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            bail!("cached metadata truncated");
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn str(&mut self) -> Result<Option<String>> {
        let n = self.u16()? as usize;
        let s = String::from_utf8_lossy(self.take(n)?).into_owned();
        Ok(if s.is_empty() { None } else { Some(s) })
    }
}

/// Packs metadata and the cover image into one blob.
pub fn encode(meta: &TrackMeta, art: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(256 + art.map_or(0, |a| a.len()));
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());

    for s in [&meta.title, &meta.artist, &meta.album, &meta.album_artist, &meta.genre, &meta.composer] {
        put_str(&mut out, s.as_deref());
    }
    for v in [meta.duration_ms, meta.sample_rate, meta.file_size, meta.mtime] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [meta.track_no, meta.disc_no, meta.year, meta.bitrate_kbps] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(meta.codec as u8);
    out.push(meta.bits);
    out.push(meta.channels);
    out.push((meta.compilation as u8) | ((meta.audiobook as u8) << 1));

    // ReplayGain is optional, so each value carries a present flag.
    for g in [meta.rg_track_db, meta.rg_album_db] {
        out.push(g.is_some() as u8);
        out.extend_from_slice(&g.unwrap_or(0.0).to_le_bytes());
    }

    let art = art.unwrap_or(&[]);
    out.extend_from_slice(&(art.len() as u32).to_le_bytes());
    out.extend_from_slice(art);
    out
}

/// Unpacks a blob. `rel_path` is supplied by the caller, since the cache is keyed by it.
pub fn decode(blob: &[u8], rel_path: &str) -> Result<(TrackMeta, Option<Vec<u8>>)> {
    if blob.len() < 6 || blob[0..4] != MAGIC {
        bail!("not cached metadata");
    }
    let mut r = Reader { data: blob, pos: 4 };
    if r.u16()? != VERSION {
        bail!("unsupported cache version");
    }

    let mut meta = TrackMeta { rel_path: rel_path.to_string(), ..Default::default() };
    meta.title = r.str()?;
    meta.artist = r.str()?;
    meta.album = r.str()?;
    meta.album_artist = r.str()?;
    meta.genre = r.str()?;
    meta.composer = r.str()?;
    meta.duration_ms = r.u32()?;
    meta.sample_rate = r.u32()?;
    meta.file_size = r.u32()?;
    meta.mtime = r.u32()?;
    meta.track_no = r.u16()?;
    meta.disc_no = r.u16()?;
    meta.year = r.u16()?;
    meta.bitrate_kbps = r.u16()?;
    meta.codec = match r.u8()? {
        1 => Codec::Mp3, 2 => Codec::Aac, 3 => Codec::Alac, 4 => Codec::Flac,
        5 => Codec::Vorbis, 6 => Codec::Opus, 7 => Codec::Wav, 8 => Codec::Aiff,
        9 => Codec::WavPack, 10 => Codec::Ape, 11 => Codec::Musepack,
        _ => Codec::Unknown,
    };
    meta.bits = r.u8()?;
    meta.channels = r.u8()?;
    let flags = r.u8()?;
    meta.compilation = flags & 1 != 0;
    meta.audiobook = flags & 2 != 0;

    for which in 0..2 {
        let present = r.u8()? != 0;
        let v = f32::from_le_bytes(r.take(4)?.try_into().unwrap());
        if present {
            if which == 0 { meta.rg_track_db = Some(v) } else { meta.rg_album_db = Some(v) }
        }
    }

    let art_len = r.u32()? as usize;
    let art = if art_len > 0 { Some(r.take(art_len)?.to_vec()) } else { None };
    Ok((meta, art))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TrackMeta {
        TrackMeta {
            rel_path: "A/B/c.flac".into(),
            title: Some("Glass Weather".into()),
            artist: Some("Kesh".into()),
            album: Some("Signal Bloom".into()),
            album_artist: None,
            genre: Some("Electronic".into()),
            composer: None,
            track_no: 3, disc_no: 1, year: 2019,
            compilation: true, audiobook: false,
            duration_ms: 252_000, sample_rate: 44_100, bits: 16, channels: 2,
            bitrate_kbps: 950, codec: Codec::Flac,
            rg_track_db: Some(-6.52), rg_album_db: None,
            file_size: 30_000_000, mtime: 1_700_000_000,
            art_source: None,
        }
    }

    #[test]
    fn round_trips_every_field() {
        let m = sample();
        let art = vec![0xFFu8, 0xD8, 1, 2, 3];
        let (back, back_art) = decode(&encode(&m, Some(&art)), &m.rel_path).unwrap();

        assert_eq!(back.title, m.title);
        assert_eq!(back.album_artist, None);
        assert_eq!(back.composer, None);
        assert_eq!((back.track_no, back.disc_no, back.year), (3, 1, 2019));
        assert_eq!(back.compilation, true);
        assert_eq!(back.audiobook, false);
        assert_eq!(back.codec, Codec::Flac);
        assert_eq!(back.duration_ms, 252_000);
        assert_eq!(back.rg_track_db, Some(-6.52));
        assert_eq!(back.rg_album_db, None);
        assert_eq!(back.file_size, 30_000_000);
        assert_eq!(back_art.as_deref(), Some(&art[..]));
    }

    #[test]
    fn handles_missing_art_and_empty_strings() {
        let mut m = sample();
        m.title = None;
        let (back, art) = decode(&encode(&m, None), "x.mp3").unwrap();
        assert_eq!(back.title, None);
        assert_eq!(back.rel_path, "x.mp3");
        assert!(art.is_none());
    }

    #[test]
    fn rejects_junk_and_truncation() {
        assert!(decode(b"junk", "a").is_err());
        let good = encode(&sample(), Some(&[1, 2, 3]));
        assert!(decode(&good[..good.len() - 2], "a").is_err());
        let mut wrong = good.clone();
        wrong[4] = 9; // version
        assert!(decode(&wrong, "a").is_err());
    }
}
