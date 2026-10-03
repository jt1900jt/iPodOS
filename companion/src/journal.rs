//! Play journal written by the device, merged back on sync.
//! See docs/ipdb-format.md; records are fixed 16-byte entries.

use std::collections::HashMap;

use anyhow::Result;

pub const RECORD_SIZE: usize = 16;

pub const T_PLAYED: u8 = 1;
pub const T_SKIPPED: u8 = 2;
pub const T_RATING: u8 = 3;
pub const T_POSITION: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: u8,
    pub uid: u32,
    pub when: u32,
    pub value: u32,
}

/// Per-track state accumulated from the journal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub plays: u32,
    pub skips: u32,
    pub last_played: u32,
    pub rating: Option<u8>,
    pub position_ms: Option<u32>,
}

/// Parses a journal. A trailing partial record, which a power cut can leave behind, is
/// ignored rather than treated as corruption.
pub fn parse(data: &[u8]) -> Vec<Event> {
    data.chunks_exact(RECORD_SIZE)
        .filter_map(|r| {
            let kind = r[0];
            if !matches!(kind, T_PLAYED | T_SKIPPED | T_RATING | T_POSITION) {
                return None; // unknown record type: skip, do not abort the merge
            }
            Some(Event {
                kind,
                uid: u32::from_le_bytes(r[4..8].try_into().unwrap()),
                when: u32::from_le_bytes(r[8..12].try_into().unwrap()),
                value: u32::from_le_bytes(r[12..16].try_into().unwrap()),
            })
        })
        .collect()
}

/// Folds events into per-track state. Later events win for rating and position.
pub fn merge(events: &[Event]) -> HashMap<u32, Stats> {
    let mut out: HashMap<u32, Stats> = HashMap::new();
    for e in events {
        let s = out.entry(e.uid).or_default();
        match e.kind {
            T_PLAYED => {
                s.plays += 1;
                s.last_played = s.last_played.max(e.when);
            }
            T_SKIPPED => {
                s.skips += 1;
                s.last_played = s.last_played.max(e.when);
            }
            T_RATING => s.rating = Some(e.value.min(5) as u8),
            T_POSITION => s.position_ms = Some(e.value),
            _ => {}
        }
    }
    out
}

/// Scrobble log in the Audioscrobbler 1.1 format, for upload to Last.fm or ListenBrainz.
/// Only completed plays are listed; `lookup` supplies artist, track and album.
pub fn scrobble_log(
    events: &[Event],
    mut lookup: impl FnMut(u32) -> Option<(String, String, String, u32)>,
) -> Result<String> {
    let mut out = String::from("#AUDIOSCROBBLER/1.1\n#TZ/UTC\n#CLIENT/iPodOS\n");
    for e in events.iter().filter(|e| e.kind == T_PLAYED) {
        if let Some((artist, track, album, length_s)) = lookup(e.uid) {
            out.push_str(&format!(
                "{artist}\t{track}\t{album}\t\t{length_s}\tL\t{}\t\n",
                e.when
            ));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: u8, uid: u32, when: u32, value: u32) -> Vec<u8> {
        let mut r = vec![0u8; RECORD_SIZE];
        r[0] = kind;
        r[4..8].copy_from_slice(&uid.to_le_bytes());
        r[8..12].copy_from_slice(&when.to_le_bytes());
        r[12..16].copy_from_slice(&value.to_le_bytes());
        r
    }

    #[test]
    fn parses_and_merges() {
        let mut data = Vec::new();
        data.extend(rec(T_PLAYED, 7, 100, 0));
        data.extend(rec(T_PLAYED, 7, 200, 0));
        data.extend(rec(T_SKIPPED, 7, 150, 0));
        data.extend(rec(T_RATING, 7, 300, 4));
        data.extend(rec(T_RATING, 7, 400, 9)); // clamped
        data.extend(rec(T_POSITION, 8, 500, 12345));

        let events = parse(&data);
        assert_eq!(events.len(), 6);
        let m = merge(&events);
        let s = &m[&7];
        assert_eq!((s.plays, s.skips, s.last_played), (2, 1, 200));
        assert_eq!(s.rating, Some(5));
        assert_eq!(m[&8].position_ms, Some(12345));
    }

    #[test]
    fn tolerates_truncation_and_unknown_types() {
        let mut data = Vec::new();
        data.extend(rec(T_PLAYED, 1, 10, 0));
        data.extend(rec(99, 2, 20, 0)); // unknown type
        data.extend(vec![0u8; 7]); // torn tail from a power cut
        let events = parse(&data);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid, 1);
    }

    #[test]
    fn scrobble_format() {
        let events = parse(&rec(T_PLAYED, 5, 1700000000, 0));
        let log = scrobble_log(&events, |_| {
            Some(("Kesh".into(), "Glass Weather".into(), "Signal Bloom".into(), 252))
        })
        .unwrap();
        assert!(log.starts_with("#AUDIOSCROBBLER/1.1"));
        let line = log.lines().last().unwrap();
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f[0], "Kesh");
        assert_eq!(f[1], "Glass Weather");
        assert_eq!(f[5], "L");
        assert_eq!(f[6], "1700000000");
    }
}
