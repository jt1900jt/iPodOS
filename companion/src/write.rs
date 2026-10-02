use crate::format::*;
use crate::model::*;

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn track_bytes(t: &TrackRec, out: &mut Vec<u8>) {
    let start = out.len();
    for v in [
        t.uid, t.path, t.title, t.artist_id, t.album_id, t.genre_id, t.composer_id,
        t.duration_ms, t.sample_rate, t.file_size, t.mtime,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [t.track_no, t.disc_no, t.year, t.bitrate_kbps] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&t.rg_track_cdb.to_le_bytes());
    out.extend_from_slice(&t.rg_album_cdb.to_le_bytes());
    out.extend_from_slice(&[t.codec, t.bits, t.channels, t.flags]);
    out.extend_from_slice(&0u32.to_le_bytes());
    debug_assert_eq!(out.len() - start, TRACK_SIZE);
}

fn album_bytes(a: &AlbumRec, out: &mut Vec<u8>) {
    let start = out.len();
    for v in [a.title, a.artist_id, a.art_id, a.tracks_first, a.tracks_count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&a.year.to_le_bytes());
    out.extend_from_slice(&[a.flags, 0]);
    for c in a.colors {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    debug_assert_eq!(out.len() - start, ALBUM_SIZE);
}

fn groups(v: &[GroupRec]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * GROUP_SIZE);
    for g in v {
        for x in [g.name, g.first, g.count, g.extra] {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    out
}

/// Serialize a library. `generation` must match the art pack written alongside it.
pub fn write_library(lib: &Library, generation: u64, created: u64) -> Vec<u8> {
    let mut trks = Vec::with_capacity(lib.tracks.len() * TRACK_SIZE);
    lib.tracks.iter().for_each(|t| track_bytes(t, &mut trks));
    let mut albm = Vec::with_capacity(lib.albums.len() * ALBUM_SIZE);
    lib.albums.iter().for_each(|a| album_bytes(a, &mut albm));
    let jump: Vec<u32> = lib.jump.iter().flatten().copied().collect();

    let sections: Vec<([u8; 4], Vec<u8>, u32)> = vec![
        (sec::STRS, lib.strings.clone(), 0),
        (sec::TRKS, trks, lib.tracks.len() as u32),
        (sec::ALBM, albm, lib.albums.len() as u32),
        (sec::ARTS, groups(&lib.artists), lib.artists.len() as u32),
        (sec::GENR, groups(&lib.genres), lib.genres.len() as u32),
        (sec::COMP, groups(&lib.composers), lib.composers.len() as u32),
        (sec::PLST, groups(&lib.playlists), lib.playlists.len() as u32),
        (sec::IALB, u32s(&lib.album_tracks), lib.album_tracks.len() as u32),
        (sec::IART, u32s(&lib.artist_albums), lib.artist_albums.len() as u32),
        (sec::IGEN, u32s(&lib.genre_artists), lib.genre_artists.len() as u32),
        (sec::ICMP, u32s(&lib.composer_tracks), lib.composer_tracks.len() as u32),
        (sec::IPLS, u32s(&lib.playlist_tracks), lib.playlist_tracks.len() as u32),
        (sec::JUMP, u32s(&jump), JUMP_ROWS as u32),
    ];

    let table_off = HEADER_SIZE as usize;
    let mut cursor = align_up(table_off + sections.len() * SECTION_ENTRY_SIZE as usize, SECTION_ALIGN);
    let mut table = Vec::new();
    let mut offsets = Vec::new();
    for (id, data, count) in &sections {
        offsets.push(cursor);
        table.extend_from_slice(id);
        table.extend_from_slice(&(cursor as u32).to_le_bytes());
        table.extend_from_slice(&(data.len() as u32).to_le_bytes());
        table.extend_from_slice(&count.to_le_bytes());
        cursor = align_up(cursor + data.len(), SECTION_ALIGN);
    }
    let file_size = cursor;

    let mut buf = vec![0u8; file_size];
    buf[table_off..table_off + table.len()].copy_from_slice(&table);
    for ((_, data, _), &off) in sections.iter().zip(&offsets) {
        buf[off..off + data.len()].copy_from_slice(data);
    }

    let crc = crc32fast::hash(&buf[HEADER_SIZE as usize..]);
    let mut h = Vec::with_capacity(HEADER_SIZE as usize);
    h.extend_from_slice(&DB_MAGIC);
    h.extend_from_slice(&VERSION_MAJOR.to_le_bytes());
    h.extend_from_slice(&VERSION_MINOR.to_le_bytes());
    h.extend_from_slice(&HEADER_SIZE.to_le_bytes());
    h.extend_from_slice(&(sections.len() as u32).to_le_bytes());
    h.extend_from_slice(&(table_off as u32).to_le_bytes());
    h.extend_from_slice(&(file_size as u32).to_le_bytes());
    h.extend_from_slice(&generation.to_le_bytes());
    h.extend_from_slice(&created.to_le_bytes());
    h.extend_from_slice(&crc.to_le_bytes());
    h.extend_from_slice(&(lib.tracks.len() as u32).to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h.resize(HEADER_SIZE as usize, 0);
    buf[..HEADER_SIZE as usize].copy_from_slice(&h);
    buf
}
