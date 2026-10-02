//! End-to-end: scan the fixture library, build, write, and read back through the validator.
//! Also leaves a built library in target/test-library for the C reader tests (device/Makefile).

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use ipdb::build::{build_library, BuildOptions};
use ipdb::format::*;
use ipdb::read::{Db, PackHeader};
use ipdb::{art, scan, write};

#[test]
fn fixture_library_round_trip() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scanned = scan::scan(&root.join("tests/fixtures/library")).unwrap();
    assert_eq!(scanned.warnings.len(), 1, "only broken.mp3 should warn: {:?}", scanned.warnings);
    assert_eq!(scanned.art.len(), 3, "embedded + folder art deduplicated by content");

    let mut lib = build_library(&scanned.tracks, &scanned.playlists, &BuildOptions::default());
    let mut pack = Cursor::new(Vec::new());
    let colors = art::write_pack(&mut pack, lib.art_sources.len(), 42, |i| scanned.art[lib.art_sources[i]].load(), |i, e| {
        panic!("art {i}: {e}")
    })
    .unwrap();
    for a in lib.albums.iter_mut().filter(|a| a.art_id != NONE) {
        a.colors = colors[a.art_id as usize];
    }
    let bytes = write::write_library(&lib, 42, 0);
    let db = Db::parse(&bytes).unwrap();
    let pack = pack.into_inner();
    let ph = PackHeader::parse(&pack, pack.len() as u64).unwrap();
    assert_eq!(ph.generation, db.generation);
    assert_eq!(ph.art_count, 3);
    assert_eq!(pack.len() as u64, ph.file_size);

    assert_eq!(db.track_count(), 10);
    assert_eq!(db.album_count(), 6);
    let album = |name: &str| (0..db.album_count()).map(|i| db.album(i)).find(|a| db.string(a.title) == name).unwrap();

    // Accented names sort under their base letter.
    assert_eq!(db.string(db.album(0).title), "Æther");

    let mix = album("Night Drive Mix");
    assert!(mix.flags & album_flags::COMPILATION != 0);
    assert_eq!(db.string(db.group(sec::ARTS, mix.artist_id as usize).name), "Various Artists");
    assert_eq!(mix.art_id, NONE);

    let nb = album("Northbound");
    assert_eq!(nb.tracks_count, 2, "CD1/CD2 folders merge into one album");
    let first = db.track(db.index(sec::IALB, nb.tracks_first as usize) as usize);
    let second = db.track(db.index(sec::IALB, nb.tracks_first as usize + 1) as usize);
    assert_eq!((first.disc_no, first.codec), (1, Codec::Aac as u8));
    assert_eq!((second.disc_no, second.codec), (2, Codec::Alac as u8));
    assert!(second.flags & track_flags::LOSSLESS != 0);
    assert_ne!(nb.colors, [0; 3]);

    let ltl = album("Low Tide Lights");
    assert_eq!(ltl.year, 2019);
    let t = db.track(db.index(sec::IALB, ltl.tracks_first as usize) as usize);
    assert_eq!(t.rg_track_cdb, -652);
    assert_eq!(t.rg_album_cdb, -710);
    assert!(t.genre_id != NONE && t.composer_id != NONE);

    let untagged = album("Unknown Album");
    let t = db.track(db.index(sec::IALB, untagged.tracks_first as usize) as usize);
    assert_eq!(db.string(t.title), "Driftwood Radio");

    assert_eq!(db.group_count(sec::PLST), 1);
    let pl = db.group(sec::PLST, 0);
    let titles: Vec<&str> = (pl.first..pl.first + pl.count)
        .map(|k| db.string(db.track(db.index(sec::IPLS, k as usize) as usize).title))
        .collect();
    assert_eq!(titles, ["Glass Weather", "Copper Sky", "Paper Moons"]);

    // Songs list is alphabetical with '#'-bucket last.
    let names: Vec<&str> = (0..db.track_count()).map(|i| db.string(db.track(i).title)).collect();
    assert_eq!(names.first(), Some(&"After the Static"));
    assert_eq!(names.last(), Some(&"Paper Moons"));

    let out = root.join("target/test-library");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("library.ipdb"), &bytes).unwrap();
    fs::write(out.join("artwork.ipap"), &pack).unwrap();
}

#[test]
fn rebuild_is_deterministic() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library");
    let a = scan::scan(&root).unwrap();
    let mut tracks_rev = a.tracks.clone();
    tracks_rev.reverse();
    let l1 = build_library(&a.tracks, &a.playlists, &BuildOptions::default());
    let l2 = build_library(&tracks_rev, &a.playlists, &BuildOptions::default());
    assert_eq!(write::write_library(&l1, 1, 0), write::write_library(&l2, 1, 0));
}
