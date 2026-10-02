//! Turns scanned track metadata into the sorted, indexed `Library` the writer serializes.

use std::collections::{BTreeSet, HashMap};

use crate::format::*;
use crate::model::*;
use crate::sortkey::{fold, sort_key, SortKey};

pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
pub const UNKNOWN_ALBUM: &str = "Unknown Album";
pub const VARIOUS_ARTISTS: &str = "Various Artists";

#[derive(Clone, Debug)]
pub struct BuildOptions {
    /// Device directory the library root maps to, e.g. "/Music".
    pub path_prefix: String,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self { path_prefix: "/Music".into() }
    }
}

struct StringPool {
    bytes: Vec<u8>,
    map: HashMap<String, u32>,
}

impl StringPool {
    fn new() -> Self {
        Self { bytes: vec![0], map: HashMap::new() }
    }

    fn intern(&mut self, s: &str) -> u32 {
        let clean: String = s.chars().filter(|&c| c != '\0').collect();
        if clean.is_empty() {
            return 0;
        }
        if let Some(&o) = self.map.get(&clean) {
            return o;
        }
        let off = self.bytes.len() as u32;
        self.bytes.extend_from_slice(clean.as_bytes());
        self.bytes.push(0);
        self.map.insert(clean, off);
        off
    }
}

/// Deduplicating name table (artists, genres, composers).
#[derive(Default)]
struct Names {
    by_fold: HashMap<String, usize>,
    display: Vec<String>,
}

impl Names {
    fn add(&mut self, name: &str) -> usize {
        let f = fold(name);
        if let Some(&i) = self.by_fold.get(&f) {
            return i;
        }
        let i = self.display.len();
        self.display.push(name.to_string());
        self.by_fold.insert(f, i);
        i
    }

    /// Returns (display order, old index -> new id).
    fn order(&self) -> (Vec<usize>, Vec<u32>) {
        let mut idx: Vec<usize> = (0..self.display.len()).collect();
        let keys: Vec<(SortKey, String)> =
            self.display.iter().map(|n| (sort_key(n), fold(n))).collect();
        idx.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
        let mut remap = vec![0u32; idx.len()];
        for (new, &old) in idx.iter().enumerate() {
            remap[old] = new as u32;
        }
        (idx, remap)
    }
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_ref()
        .map(|x| x.replace('\0', "").trim().to_string())
        .filter(|x| !x.is_empty())
}

fn parent_dir(rel: &str) -> &str {
    rel.rfind('/').map(|i| &rel[..i]).unwrap_or("")
}

fn file_stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rfind('.').filter(|&i| i > 0).map(|i| &name[..i]).unwrap_or(name)
}

fn fnv1a32(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn gain_cdb(db: Option<f32>) -> (i16, bool) {
    match db {
        Some(v) if v.is_finite() => ((v * 100.0).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16, true),
        _ => (0, false),
    }
}

fn device_path(prefix: &str, rel: &str) -> String {
    let p = prefix.trim_end_matches('/');
    let p = if p.starts_with('/') { p.to_string() } else { format!("/{p}") };
    if p == "/" {
        format!("/{rel}")
    } else {
        format!("{p}/{rel}")
    }
}

/// row[b] = first index whose bucket >= b, or `count`.
fn jump_row(buckets: &[u8]) -> [u32; JUMP_BUCKETS] {
    let mut row = [buckets.len() as u32; JUMP_BUCKETS];
    let mut i = 0usize;
    for (b, slot) in row.iter_mut().enumerate() {
        while i < buckets.len() && (buckets[i] as usize) < b {
            i += 1;
        }
        *slot = i as u32;
    }
    row
}

fn most_common<T: Clone + Eq + std::hash::Hash>(items: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: HashMap<T, usize> = HashMap::new();
    let mut first_seen: Vec<T> = Vec::new();
    for it in items {
        let c = counts.entry(it.clone()).or_insert(0);
        if *c == 0 {
            first_seen.push(it);
        }
        *c += 1;
    }
    // Ties resolve to the earliest seen, keeping output deterministic.
    let mut best: Option<T> = None;
    for it in first_seen {
        if best.as_ref().map_or(true, |b| counts[&it] > counts[b]) {
            best = Some(it);
        }
    }
    best
}

struct Norm {
    title: String,
    artist: String,
    album: String,
    album_artist_tag: Option<String>,
    genre: Option<String>,
    composer: Option<String>,
}

pub fn build_library(input: &[TrackMeta], playlists: &[PlaylistMeta], opts: &BuildOptions) -> Library {
    // Deterministic processing order regardless of caller order.
    let mut order: Vec<usize> = (0..input.len()).collect();
    order.sort_by(|&a, &b| input[a].rel_path.cmp(&input[b].rel_path));
    let tracks: Vec<&TrackMeta> = order.iter().map(|&i| &input[i]).collect();
    let n = tracks.len();

    let norm: Vec<Norm> = tracks
        .iter()
        .map(|t| Norm {
            title: clean(&t.title).unwrap_or_else(|| file_stem(&t.rel_path).to_string()),
            artist: clean(&t.artist).unwrap_or_else(|| UNKNOWN_ARTIST.into()),
            album: clean(&t.album).unwrap_or_else(|| UNKNOWN_ALBUM.into()),
            album_artist_tag: clean(&t.album_artist),
            genre: clean(&t.genre),
            composer: clean(&t.composer),
        })
        .collect();

    let mut artists = Names::default();
    let mut genres = Names::default();
    let mut composers = Names::default();
    let track_artist: Vec<usize> = norm.iter().map(|x| artists.add(&x.artist)).collect();
    let track_genre: Vec<Option<usize>> = norm.iter().map(|x| x.genre.as_deref().map(|g| genres.add(g))).collect();
    let track_composer: Vec<Option<usize>> =
        norm.iter().map(|x| x.composer.as_deref().map(|c| composers.add(c))).collect();

    // Albums, stage 1: group by (album, directory) and decide the album artist per group.
    let mut stage1: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut stage1_order: Vec<(String, String)> = Vec::new();
    for i in 0..n {
        let key = (fold(&norm[i].album), parent_dir(&tracks[i].rel_path).to_string());
        stage1.entry(key.clone()).or_insert_with(|| {
            stage1_order.push(key.clone());
            Vec::new()
        }).push(i);
    }

    struct AlbumGroup {
        title: String,
        album_artist: usize,
        compilation: bool,
        members: Vec<usize>,
    }
    let mut groups: Vec<AlbumGroup> = Vec::new();
    let mut by_identity: HashMap<(String, usize), usize> = HashMap::new();
    for key in &stage1_order {
        let members = &stage1[key];
        let tagged_aa = most_common(members.iter().filter_map(|&i| norm[i].album_artist_tag.clone()));
        let distinct: BTreeSet<usize> = members.iter().map(|&i| track_artist[i]).collect();
        let flagged = members.iter().any(|&i| tracks[i].compilation);
        let (aa_name, comp) = match tagged_aa {
            Some(a) => {
                let comp = flagged || fold(&a) == fold(VARIOUS_ARTISTS);
                (a, comp)
            }
            None if flagged || distinct.len() > 1 => (VARIOUS_ARTISTS.to_string(), true),
            None => (norm[members[0]].artist.clone(), false),
        };
        let aa = artists.add(&aa_name);
        // Stage 2: merge groups sharing (album, album artist), e.g. CD1/CD2 subfolders.
        let ident = (key.0.clone(), aa);
        match by_identity.get(&ident) {
            Some(&g) => {
                groups[g].members.extend_from_slice(members);
                groups[g].compilation |= comp;
            }
            None => {
                by_identity.insert(ident, groups.len());
                groups.push(AlbumGroup {
                    title: norm[members[0]].album.clone(),
                    album_artist: aa,
                    compilation: comp,
                    members: members.clone(),
                });
            }
        }
    }
    let mut track_group = vec![0usize; n];
    for (g, ag) in groups.iter().enumerate() {
        for &m in &ag.members {
            track_group[m] = g;
        }
    }

    let (artist_order, artist_id) = artists.order();
    let (genre_order, genre_id) = genres.order();
    let (composer_order, composer_id) = composers.order();

    // Track order: title, then artist, album, disc, track, path.
    let title_keys: Vec<SortKey> = norm.iter().map(|x| sort_key(&x.title)).collect();
    let artist_keys: Vec<SortKey> = artists.display.iter().map(|a| sort_key(a)).collect();
    let album_keys: Vec<SortKey> = groups.iter().map(|g| sort_key(&g.title)).collect();
    let mut tord: Vec<usize> = (0..n).collect();
    tord.sort_by(|&a, &b| {
        title_keys[a]
            .cmp(&title_keys[b])
            .then_with(|| artist_keys[track_artist[a]].cmp(&artist_keys[track_artist[b]]))
            .then_with(|| album_keys[track_group[a]].cmp(&album_keys[track_group[b]]))
            .then_with(|| tracks[a].disc_no.cmp(&tracks[b].disc_no))
            .then_with(|| tracks[a].track_no.cmp(&tracks[b].track_no))
            .then_with(|| tracks[a].rel_path.cmp(&tracks[b].rel_path))
    });
    let mut track_id = vec![0u32; n];
    for (new, &old) in tord.iter().enumerate() {
        track_id[old] = new as u32;
    }

    // Album order: title, then album artist.
    let mut gord: Vec<usize> = (0..groups.len()).collect();
    gord.sort_by(|&a, &b| {
        album_keys[a]
            .cmp(&album_keys[b])
            .then_with(|| artist_keys[groups[a].album_artist].cmp(&artist_keys[groups[b].album_artist]))
            .then_with(|| fold(&groups[a].title).cmp(&fold(&groups[b].title)))
    });
    let mut album_id = vec![0u32; groups.len()];
    for (new, &old) in gord.iter().enumerate() {
        album_id[old] = new as u32;
    }

    let mut lib = Library::default();
    let mut pool = StringPool::new();

    // Tracks.
    let mut used_uids: BTreeSet<u32> = BTreeSet::new();
    let mut uids = vec![0u32; n];
    for i in 0..n {
        let mut u = fnv1a32(&tracks[i].rel_path);
        while u == NONE || used_uids.contains(&u) {
            u = u.wrapping_add(1);
        }
        used_uids.insert(u);
        uids[i] = u;
    }
    for &i in &tord {
        let t = tracks[i];
        let (rgt, has_t) = gain_cdb(t.rg_track_db);
        let (rga, has_a) = gain_cdb(t.rg_album_db);
        let mut flags = 0u8;
        if has_t { flags |= track_flags::RG_TRACK; }
        if has_a { flags |= track_flags::RG_ALBUM; }
        if groups[track_group[i]].compilation { flags |= track_flags::COMPILATION; }
        if t.codec.is_lossless() { flags |= track_flags::LOSSLESS; }
        if t.audiobook { flags |= track_flags::AUDIOBOOK; }
        lib.tracks.push(TrackRec {
            uid: uids[i],
            path: pool.intern(&device_path(&opts.path_prefix, &t.rel_path)),
            title: pool.intern(&norm[i].title),
            artist_id: artist_id[track_artist[i]],
            album_id: album_id[track_group[i]],
            genre_id: track_genre[i].map(|g| genre_id[g]).unwrap_or(NONE),
            composer_id: track_composer[i].map(|c| composer_id[c]).unwrap_or(NONE),
            duration_ms: t.duration_ms,
            sample_rate: t.sample_rate,
            file_size: t.file_size,
            mtime: t.mtime,
            track_no: t.track_no,
            disc_no: t.disc_no,
            year: t.year,
            bitrate_kbps: t.bitrate_kbps,
            rg_track_cdb: rgt,
            rg_album_cdb: rga,
            codec: t.codec as u8,
            bits: if t.codec.is_lossless() { t.bits } else { 0 },
            channels: t.channels,
            flags,
        });
    }

    // Albums, their track lists and art.
    let mut art_of_source: HashMap<usize, u32> = HashMap::new();
    for &g in &gord {
        let ag = &groups[g];
        let mut members = ag.members.clone();
        members.sort_by(|&a, &b| {
            tracks[a]
                .disc_no
                .cmp(&tracks[b].disc_no)
                .then(tracks[a].track_no.cmp(&tracks[b].track_no))
                .then_with(|| title_keys[a].cmp(&title_keys[b]))
                .then_with(|| tracks[a].rel_path.cmp(&tracks[b].rel_path))
        });
        let first = lib.album_tracks.len() as u32;
        lib.album_tracks.extend(members.iter().map(|&m| track_id[m]));
        let year = most_common(members.iter().map(|&m| tracks[m].year).filter(|&y| y != 0)).unwrap_or(0);
        let art_id = members
            .iter()
            .find_map(|&m| tracks[m].art_source)
            .map(|src| {
                *art_of_source.entry(src).or_insert_with(|| {
                    lib.art_sources.push(src);
                    (lib.art_sources.len() - 1) as u32
                })
            })
            .unwrap_or(NONE);
        lib.albums.push(AlbumRec {
            title: pool.intern(&ag.title),
            artist_id: artist_id[ag.album_artist],
            art_id,
            tracks_first: first,
            tracks_count: members.len() as u32,
            year,
            flags: if ag.compilation { album_flags::COMPILATION } else { 0 },
            colors: [0; 3],
        });
    }

    // Artists -> albums (album artist or performer), by year then title.
    let n_artists = artist_order.len();
    let mut artist_albums: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n_artists];
    let mut artist_tracks = vec![0u32; n_artists];
    for (g, ag) in groups.iter().enumerate() {
        artist_albums[artist_id[ag.album_artist] as usize].insert(album_id[g]);
        for &m in &ag.members {
            artist_albums[artist_id[track_artist[m]] as usize].insert(album_id[g]);
        }
    }
    for i in 0..n {
        artist_tracks[artist_id[track_artist[i]] as usize] += 1;
    }
    for (new, &old) in artist_order.iter().enumerate() {
        let mut list: Vec<u32> = artist_albums[new].iter().copied().collect();
        list.sort_by_key(|&a| {
            let y = lib.albums[a as usize].year;
            (y == 0, y, a)
        });
        let first = lib.artist_albums.len() as u32;
        lib.artist_albums.extend_from_slice(&list);
        lib.artists.push(GroupRec {
            name: pool.intern(&artists.display[old]),
            first,
            count: list.len() as u32,
            extra: artist_tracks[new],
        });
    }

    // Genres -> artists.
    let mut genre_artists: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); genre_order.len()];
    let mut genre_tracks = vec![0u32; genre_order.len()];
    for i in 0..n {
        if let Some(g) = track_genre[i] {
            let gid = genre_id[g] as usize;
            genre_artists[gid].insert(artist_id[track_artist[i]]);
            genre_tracks[gid] += 1;
        }
    }
    for (new, &old) in genre_order.iter().enumerate() {
        let first = lib.genre_artists.len() as u32;
        lib.genre_artists.extend(genre_artists[new].iter().copied());
        lib.genres.push(GroupRec {
            name: pool.intern(&genres.display[old]),
            first,
            count: genre_artists[new].len() as u32,
            extra: genre_tracks[new],
        });
    }

    // Composers -> tracks (title order == track id order).
    let mut composer_tracks: Vec<Vec<u32>> = vec![Vec::new(); composer_order.len()];
    for i in 0..n {
        if let Some(c) = track_composer[i] {
            composer_tracks[composer_id[c] as usize].push(track_id[i]);
        }
    }
    for (new, &old) in composer_order.iter().enumerate() {
        let list = &mut composer_tracks[new];
        list.sort_unstable();
        let first = lib.composer_tracks.len() as u32;
        lib.composer_tracks.extend_from_slice(list);
        lib.composers.push(GroupRec {
            name: pool.intern(&composers.display[old]),
            first,
            count: list.len() as u32,
            extra: 0,
        });
    }

    // Playlists, sorted by name; entries in file order, unresolved entries dropped.
    let by_path: HashMap<&str, u32> = (0..n).map(|i| (tracks[i].rel_path.as_str(), track_id[i])).collect();
    let mut pls: Vec<&PlaylistMeta> = playlists.iter().collect();
    pls.sort_by(|a, b| (sort_key(&a.name), fold(&a.name)).cmp(&(sort_key(&b.name), fold(&b.name))));
    for p in pls {
        let first = lib.playlist_tracks.len() as u32;
        lib.playlist_tracks.extend(p.entries.iter().filter_map(|e| by_path.get(e.as_str()).copied()));
        lib.playlists.push(GroupRec {
            name: pool.intern(&p.name),
            first,
            count: lib.playlist_tracks.len() as u32 - first,
            extra: 0,
        });
    }

    // Jump rows.
    let tb: Vec<u8> = tord.iter().map(|&i| title_keys[i].bucket).collect();
    let ab: Vec<u8> = gord.iter().map(|&g| album_keys[g].bucket).collect();
    let rb: Vec<u8> = artist_order.iter().map(|&a| artist_keys[a].bucket).collect();
    let gb: Vec<u8> = genre_order.iter().map(|&g| sort_key(&genres.display[g]).bucket).collect();
    let cb: Vec<u8> = composer_order.iter().map(|&c| sort_key(&composers.display[c]).bucket).collect();
    lib.jump[JUMP_SONGS] = jump_row(&tb);
    lib.jump[JUMP_ALBUMS] = jump_row(&ab);
    lib.jump[JUMP_ARTISTS] = jump_row(&rb);
    lib.jump[JUMP_GENRES] = jump_row(&gb);
    lib.jump[JUMP_COMPOSERS] = jump_row(&cb);

    lib.strings = pool.bytes;
    lib
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(path: &str, title: &str, artist: &str, album: &str, tn: u16) -> TrackMeta {
        TrackMeta {
            rel_path: path.into(),
            title: Some(title.into()),
            artist: Some(artist.into()),
            album: Some(album.into()),
            track_no: tn,
            ..Default::default()
        }
    }

    #[test]
    fn jump_rows() {
        assert_eq!(jump_row(&[0, 0, 2, 26])[0..4], [0, 2, 2, 3]);
        assert_eq!(jump_row(&[0, 0, 2, 26])[26], 3);
        assert_eq!(jump_row(&[])[0], 0);
    }

    #[test]
    fn compilation_and_multidisc() {
        let input = vec![
            t("Comp/01.mp3", "One", "A", "Mix", 1),
            t("Comp/02.mp3", "Two", "B", "Mix", 2),
            t("Band/Album/CD1/01.flac", "Alpha", "Band", "Album", 1),
            TrackMeta { disc_no: 2, ..t("Band/Album/CD2/01.flac", "Beta", "Band", "Album", 1) },
        ];
        let lib = build_library(&input, &[], &BuildOptions::default());
        assert_eq!(lib.albums.len(), 2);
        let mix = lib.albums.iter().find(|a| a.flags & album_flags::COMPILATION != 0).unwrap();
        assert_eq!(mix.tracks_count, 2);
        let album = lib.albums.iter().find(|a| a.flags == 0).unwrap();
        assert_eq!(album.tracks_count, 2);
        // Disc 1 plays before disc 2.
        let first = lib.album_tracks[album.tracks_first as usize] as usize;
        assert_eq!(lib.tracks[first].disc_no, 0);
    }
}
