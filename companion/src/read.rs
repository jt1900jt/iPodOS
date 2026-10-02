//! Validating reader. Mirrors the device-side checks in device/ipdb/ipdb.c so the
//! companion can verify what it wrote and read back device state.

use std::collections::HashMap;

use anyhow::{bail, ensure, Context, Result};

use crate::format::*;
use crate::model::*;

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn i16_at(b: &[u8], o: usize) -> i16 {
    u16_at(b, o) as i16
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

pub struct Db<'a> {
    pub generation: u64,
    pub created: u64,
    pub version_minor: u16,
    sections: HashMap<[u8; 4], (&'a [u8], u32)>,
}

impl<'a> Db<'a> {
    pub fn parse(buf: &'a [u8]) -> Result<Self> {
        ensure!(buf.len() >= HEADER_SIZE as usize, "file shorter than header");
        ensure!(buf[0..4] == DB_MAGIC, "bad magic");
        ensure!(u16_at(buf, 4) == VERSION_MAJOR, "unsupported major version {}", u16_at(buf, 4));
        let header_size = u32_at(buf, 8) as usize;
        ensure!(header_size == HEADER_SIZE as usize, "bad header size");
        let count = u32_at(buf, 12) as usize;
        let table = u32_at(buf, 16) as usize;
        let file_size = u32_at(buf, 20) as usize;
        ensure!(file_size <= buf.len(), "file truncated: header says {file_size}, have {}", buf.len());
        ensure!(count <= 4096, "absurd section count");
        let table_end = table
            .checked_add(count * SECTION_ENTRY_SIZE as usize)
            .context("section table overflow")?;
        ensure!(table >= header_size && table_end <= file_size, "section table out of bounds");
        let crc = crc32fast::hash(&buf[header_size..file_size]);
        ensure!(crc == u32_at(buf, 40), "crc mismatch");

        let mut sections = HashMap::new();
        for i in 0..count {
            let e = table + i * SECTION_ENTRY_SIZE as usize;
            let id: [u8; 4] = buf[e..e + 4].try_into().unwrap();
            let off = u32_at(buf, e + 4) as usize;
            let size = u32_at(buf, e + 8) as usize;
            let cnt = u32_at(buf, e + 12);
            ensure!(off % SECTION_ALIGN == 0, "section {} misaligned", fourcc_str(&id));
            let end = off.checked_add(size).context("section overflow")?;
            ensure!(off >= table_end && end <= file_size, "section {} out of bounds", fourcc_str(&id));
            sections.insert(id, (&buf[off..end], cnt));
        }

        let db = Db {
            generation: u64_at(buf, 24),
            created: u64_at(buf, 32),
            version_minor: u16_at(buf, 6),
            sections,
        };
        db.validate()?;
        ensure!(u32_at(buf, 44) as usize == db.track_count(), "track_count mismatch");
        Ok(db)
    }

    fn sec(&self, id: [u8; 4]) -> (&'a [u8], u32) {
        self.sections[&id]
    }

    fn validate(&self) -> Result<()> {
        for id in sec::ALL {
            ensure!(self.sections.contains_key(&id), "missing section {}", fourcc_str(&id));
        }
        let rec = |id: [u8; 4], size: usize| -> Result<()> {
            let (d, c) = self.sec(id);
            ensure!(d.len() == c as usize * size, "section {} size/count mismatch", fourcc_str(&id));
            Ok(())
        };
        rec(sec::TRKS, TRACK_SIZE)?;
        rec(sec::ALBM, ALBUM_SIZE)?;
        for id in [sec::ARTS, sec::GENR, sec::COMP, sec::PLST] {
            rec(id, GROUP_SIZE)?;
        }
        for id in [sec::IALB, sec::IART, sec::IGEN, sec::ICMP, sec::IPLS] {
            rec(id, 4)?;
        }
        rec(sec::JUMP, JUMP_BUCKETS * 4)?;
        ensure!(self.sec(sec::JUMP).1 as usize == JUMP_ROWS, "JUMP row count");

        let strs = self.sec(sec::STRS).0;
        ensure!(!strs.is_empty() && strs[0] == 0 && *strs.last().unwrap() == 0, "string pool not NUL-bounded");
        let s_ok = |o: u32| (o as usize) < strs.len();

        let (nt, na, nr, ng, nc) = (
            self.track_count() as u32,
            self.album_count() as u32,
            self.group_count(sec::ARTS) as u32,
            self.group_count(sec::GENR) as u32,
            self.group_count(sec::COMP) as u32,
        );
        for i in 0..nt as usize {
            let t = self.track(i);
            ensure!(s_ok(t.path) && s_ok(t.title), "track {i}: string out of range");
            ensure!(t.artist_id < nr, "track {i}: artist out of range");
            ensure!(t.album_id < na, "track {i}: album out of range");
            ensure!(t.genre_id == NONE || t.genre_id < ng, "track {i}: genre out of range");
            ensure!(t.composer_id == NONE || t.composer_id < nc, "track {i}: composer out of range");
        }
        let csr = |idx: [u8; 4], first: u32, count: u32, max: u32, what: &str| -> Result<()> {
            let n = self.sec(idx).1;
            let end = first.checked_add(count).context("csr overflow")?;
            ensure!(end <= n, "{what}: list out of range");
            for k in first..end {
                ensure!(self.index(idx, k as usize) < max, "{what}: entry out of range");
            }
            Ok(())
        };
        for i in 0..na as usize {
            let a = self.album(i);
            ensure!(s_ok(a.title), "album {i}: title out of range");
            ensure!(a.artist_id < nr, "album {i}: artist out of range");
            csr(sec::IALB, a.tracks_first, a.tracks_count, nt, "album tracks")?;
        }
        for (id, idx, max) in [
            (sec::ARTS, sec::IART, na),
            (sec::GENR, sec::IGEN, nr),
            (sec::COMP, sec::ICMP, nt),
            (sec::PLST, sec::IPLS, nt),
        ] {
            for i in 0..self.group_count(id) {
                let g = self.group(id, i);
                ensure!(s_ok(g.name), "{} {i}: name out of range", fourcc_str(&id));
                csr(idx, g.first, g.count, max, &fourcc_str(&id))?;
            }
        }
        let counts = [nt, na, nr, ng, nc];
        for (row, &cnt) in counts.iter().enumerate() {
            let j = self.jump_row(row);
            for b in 0..JUMP_BUCKETS {
                ensure!(j[b] <= cnt, "jump row {row} out of range");
                if b > 0 {
                    ensure!(j[b] >= j[b - 1], "jump row {row} not monotonic");
                }
            }
        }
        Ok(())
    }

    pub fn track_count(&self) -> usize {
        self.sec(sec::TRKS).1 as usize
    }
    pub fn album_count(&self) -> usize {
        self.sec(sec::ALBM).1 as usize
    }
    pub fn group_count(&self, id: [u8; 4]) -> usize {
        self.sec(id).1 as usize
    }

    pub fn string(&self, off: u32) -> &'a str {
        let s = self.sec(sec::STRS).0;
        let start = off as usize;
        let end = s[start..].iter().position(|&b| b == 0).map(|p| start + p).unwrap_or(s.len());
        std::str::from_utf8(&s[start..end]).unwrap_or("\u{FFFD}")
    }

    pub fn index(&self, id: [u8; 4], i: usize) -> u32 {
        u32_at(self.sec(id).0, i * 4)
    }

    pub fn jump_row(&self, row: usize) -> [u32; JUMP_BUCKETS] {
        let d = self.sec(sec::JUMP).0;
        let mut out = [0u32; JUMP_BUCKETS];
        for (b, v) in out.iter_mut().enumerate() {
            *v = u32_at(d, (row * JUMP_BUCKETS + b) * 4);
        }
        out
    }

    pub fn track(&self, i: usize) -> TrackRec {
        let b = &self.sec(sec::TRKS).0[i * TRACK_SIZE..(i + 1) * TRACK_SIZE];
        TrackRec {
            uid: u32_at(b, 0),
            path: u32_at(b, 4),
            title: u32_at(b, 8),
            artist_id: u32_at(b, 12),
            album_id: u32_at(b, 16),
            genre_id: u32_at(b, 20),
            composer_id: u32_at(b, 24),
            duration_ms: u32_at(b, 28),
            sample_rate: u32_at(b, 32),
            file_size: u32_at(b, 36),
            mtime: u32_at(b, 40),
            track_no: u16_at(b, 44),
            disc_no: u16_at(b, 46),
            year: u16_at(b, 48),
            bitrate_kbps: u16_at(b, 50),
            rg_track_cdb: i16_at(b, 52),
            rg_album_cdb: i16_at(b, 54),
            codec: b[56],
            bits: b[57],
            channels: b[58],
            flags: b[59],
        }
    }

    pub fn album(&self, i: usize) -> AlbumRec {
        let b = &self.sec(sec::ALBM).0[i * ALBUM_SIZE..(i + 1) * ALBUM_SIZE];
        AlbumRec {
            title: u32_at(b, 0),
            artist_id: u32_at(b, 4),
            art_id: u32_at(b, 8),
            tracks_first: u32_at(b, 12),
            tracks_count: u32_at(b, 16),
            year: u16_at(b, 20),
            flags: b[22],
            colors: [u16_at(b, 24), u16_at(b, 26), u16_at(b, 28)],
        }
    }

    pub fn group(&self, id: [u8; 4], i: usize) -> GroupRec {
        let b = &self.sec(id).0[i * GROUP_SIZE..(i + 1) * GROUP_SIZE];
        GroupRec { name: u32_at(b, 0), first: u32_at(b, 4), count: u32_at(b, 8), extra: u32_at(b, 12) }
    }
}

#[derive(Debug, Clone)]
pub struct PackClass {
    pub id: [u8; 4],
    pub width: u32,
    pub height: u32,
    pub offset: u64,
}

#[derive(Debug, Clone)]
pub struct PackHeader {
    pub art_count: u32,
    pub data_offset: u32,
    pub generation: u64,
    pub file_size: u64,
    pub classes: Vec<PackClass>,
}

impl PackHeader {
    /// Parse and check the header + class table. `head` must contain at least that much of the file.
    pub fn parse(head: &[u8], actual_len: u64) -> Result<Self> {
        ensure!(head.len() >= HEADER_SIZE as usize, "short art pack");
        ensure!(head[0..4] == ART_MAGIC, "bad art pack magic");
        ensure!(u16_at(head, 4) == VERSION_MAJOR, "unsupported art pack version");
        ensure!(u32_at(head, 8) == HEADER_SIZE, "bad art pack header size");
        let n = u32_at(head, 12) as usize;
        ensure!(n <= 64, "absurd class count");
        let table_end = HEADER_SIZE as usize + n * ART_CLASS_ENTRY_SIZE as usize;
        ensure!(head.len() >= table_end, "short art pack class table");
        let mut copy = head[..table_end].to_vec();
        copy[40..44].fill(0);
        ensure!(crc32fast::hash(&copy) == u32_at(head, 40), "art pack header crc mismatch");
        let art_count = u32_at(head, 16);
        let data_offset = u32_at(head, 20);
        let file_size = u64_at(head, 32);
        ensure!(file_size <= actual_len, "art pack truncated");
        ensure!(data_offset as usize >= table_end, "art data overlaps header");
        let mut classes = Vec::new();
        for i in 0..n {
            let e = HEADER_SIZE as usize + i * ART_CLASS_ENTRY_SIZE as usize;
            let c = PackClass {
                id: head[e..e + 4].try_into().unwrap(),
                width: u16_at(head, e + 4) as u32,
                height: u16_at(head, e + 6) as u32,
                offset: u64_at(head, e + 8),
            };
            let slot = c.width as u64 * c.height as u64 * 2;
            let end = c.offset.checked_add(slot * art_count as u64);
            match end {
                Some(end) if c.offset >= data_offset as u64 && end <= file_size => {}
                _ => bail!("class {} out of bounds", fourcc_str(&c.id)),
            }
            classes.push(c);
        }
        Ok(PackHeader { art_count, data_offset, generation: u64_at(head, 24), file_size, classes })
    }

    pub fn class(&self, id: &[u8; 4]) -> Option<&PackClass> {
        self.classes.iter().find(|c| &c.id == id)
    }
}
