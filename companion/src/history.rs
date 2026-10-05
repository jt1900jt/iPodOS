//! Cumulative listening history, kept on the host.
//!
//! The device journal is a log of events since the last sync and is cleared when read.
//! This is the running total it feeds into: plays, skips, ratings and timestamps per
//! track, carried across syncs so smart playlists have something to work from.
//!
//! Tracks are keyed by the same uid the database uses, derived from the library-relative
//! path, so history survives a rebuild as long as the file stays put.

use std::collections::HashMap;

use anyhow::{bail, Result};

use crate::journal;

pub const MAGIC: [u8; 4] = *b"IPHS";
pub const RECORD: usize = 24;
const HEADER: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub plays: u32,
    pub skips: u32,
    pub last_played: u32,
    pub first_seen: u32,
    pub rating: u8,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub tracks: HashMap<u32, Stats>,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl History {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Ok(Self::default());
        }
        if data.len() < HEADER || data[0..4] != MAGIC {
            bail!("not a history file");
        }
        if u32_at(data, 4) != 1 {
            bail!("unsupported history version");
        }
        let count = u32_at(data, 8) as usize;
        let body = &data[HEADER..];
        if body.len() < count * RECORD {
            bail!("history truncated");
        }
        let mut tracks = HashMap::with_capacity(count);
        for r in body.chunks_exact(RECORD).take(count) {
            tracks.insert(
                u32_at(r, 0),
                Stats {
                    plays: u32_at(r, 4),
                    skips: u32_at(r, 8),
                    last_played: u32_at(r, 12),
                    first_seen: u32_at(r, 16),
                    rating: r[20],
                },
            );
        }
        Ok(Self { tracks })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.tracks.len() * RECORD);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(self.tracks.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        // Sorted so the file is reproducible and diffable.
        let mut uids: Vec<&u32> = self.tracks.keys().collect();
        uids.sort_unstable();
        for uid in uids {
            let s = &self.tracks[uid];
            out.extend_from_slice(&uid.to_le_bytes());
            out.extend_from_slice(&s.plays.to_le_bytes());
            out.extend_from_slice(&s.skips.to_le_bytes());
            out.extend_from_slice(&s.last_played.to_le_bytes());
            out.extend_from_slice(&s.first_seen.to_le_bytes());
            out.push(s.rating);
            out.extend_from_slice(&[0, 0, 0]);
        }
        out
    }

    /// Folds a device journal into the running totals.
    pub fn merge_journal(&mut self, events: &[journal::Event]) {
        for e in events {
            let s = self.tracks.entry(e.uid).or_default();
            match e.kind {
                journal::T_PLAYED => {
                    s.plays += 1;
                    s.last_played = s.last_played.max(e.when);
                }
                journal::T_SKIPPED => {
                    s.skips += 1;
                    s.last_played = s.last_played.max(e.when);
                }
                // A rating is the user's latest word on the track, not a running total.
                journal::T_RATING => s.rating = e.value.min(5) as u8,
                _ => {}
            }
        }
    }

    /// Records tracks seen for the first time, so "recently added" has a date that
    /// reflects when the track entered the library rather than the file's mtime, which
    /// copying and tagging both disturb.
    pub fn note_seen(&mut self, uids: impl Iterator<Item = u32>, now: u32) {
        for uid in uids {
            let s = self.tracks.entry(uid).or_default();
            if s.first_seen == 0 {
                s.first_seen = now;
            }
        }
    }

    pub fn get(&self, uid: u32) -> Stats {
        self.tracks.get(&uid).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: u8, uid: u32, when: u32, value: u32) -> journal::Event {
        journal::Event { kind, uid, when, value }
    }

    #[test]
    fn accumulates_across_syncs() {
        let mut h = History::default();
        h.merge_journal(&[ev(journal::T_PLAYED, 7, 100, 0), ev(journal::T_SKIPPED, 7, 90, 0)]);
        h.merge_journal(&[ev(journal::T_PLAYED, 7, 200, 0), ev(journal::T_RATING, 7, 150, 4)]);

        let s = h.get(7);
        assert_eq!((s.plays, s.skips), (2, 1));
        assert_eq!(s.last_played, 200);
        assert_eq!(s.rating, 4);
    }

    #[test]
    fn round_trips_through_the_file() {
        let mut h = History::default();
        h.merge_journal(&[ev(journal::T_PLAYED, 3, 50, 0), ev(journal::T_RATING, 9, 60, 5)]);
        h.note_seen([3u32, 9, 11].into_iter(), 1234);

        let bytes = h.write();
        let back = History::parse(&bytes).unwrap();
        assert_eq!(back.tracks.len(), 3);
        assert_eq!(back.get(3).plays, 1);
        assert_eq!(back.get(9).rating, 5);
        assert_eq!(back.get(11).first_seen, 1234);
        // writing is deterministic
        assert_eq!(bytes, back.write());
    }

    #[test]
    fn first_seen_is_not_overwritten() {
        let mut h = History::default();
        h.note_seen([5u32].into_iter(), 100);
        h.note_seen([5u32].into_iter(), 900);
        assert_eq!(h.get(5).first_seen, 100);
    }

    #[test]
    fn rejects_junk_but_accepts_empty() {
        assert!(History::parse(&[]).unwrap().tracks.is_empty());
        assert!(History::parse(b"nope").is_err());
        let mut bad = History::default().write();
        bad[8] = 9; // claims records that are not there
        assert!(History::parse(&bad).is_err());
    }
}
