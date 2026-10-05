//! Exercises the C ABI the browser companion calls. The wasm packaging is built on the
//! developer's machine, but the logic behind it is covered here.

use std::ffi::c_void;
use std::path::PathBuf;

use ipdb::read::{Db, PackHeader};
use ipdb::wasm::*;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library").join(rel)
}

unsafe fn add(rel: &str, path_on_disk: &str) -> i32 {
    let data = std::fs::read(fixture(path_on_disk)).unwrap();
    ipdb_add_track(rel.as_ptr(), rel.len(), data.as_ptr(), data.len(), 1_700_000_000)
}

unsafe fn db_bytes() -> Vec<u8> {
    std::slice::from_raw_parts(ipdb_db_ptr(), ipdb_db_len()).to_vec()
}

unsafe fn art_bytes() -> Vec<u8> {
    std::slice::from_raw_parts(ipdb_art_ptr(), ipdb_art_len()).to_vec()
}

#[test]
fn builds_a_library_through_the_c_abi() {
    unsafe {
        ipdb_reset();
        assert_eq!(add("Halcyon Drift/Low Tide Lights/01 After the Static.flac",
                       "Halcyon Drift/Low Tide Lights/01 After the Static.flac"), 1);
        assert_eq!(add("Halcyon Drift/Low Tide Lights/02 Half Light.flac",
                       "Halcyon Drift/Low Tide Lights/02 Half Light.flac"), 1);
        assert_eq!(add("Mara Vell/Paper Moons/01 Barrow Lane.mp3",
                       "Mara Vell/Paper Moons/01 Barrow Lane.mp3"), 1);
        assert_eq!(ipdb_track_count(), 3);

        let name = "Mix";
        let entries = "Mara Vell/Paper Moons/01 Barrow Lane.mp3\nmissing.flac\n";
        ipdb_add_playlist(name.as_ptr(), name.len(), entries.as_ptr(), entries.len());

        let prefix = "/Music";
        assert_eq!(ipdb_build(prefix.as_ptr(), prefix.len(), 4242), 3);

        let raw = db_bytes();
        let db = Db::parse(&raw).expect("database must validate");
        assert_eq!(db.track_count(), 3);
        assert_eq!(db.generation, 4242);
        assert_eq!(db.group_count(ipdb::format::sec::PLST), 1);
        // the unresolved playlist entry is dropped, the real one kept
        assert_eq!(db.group(ipdb::format::sec::PLST, 0).count, 1);

        let pack = art_bytes();
        let ph = PackHeader::parse(&pack, pack.len() as u64).unwrap();
        assert_eq!(ph.generation, db.generation);
        // the two FLACs share one embedded cover, the MP3 has its own
        assert_eq!(ph.art_count, 2);

        // paths carry the prefix
        let t = db.track(0);
        assert!(db.string(t.path).starts_with("/Music/"));
    }
}

#[test]
fn reports_unparseable_files_without_failing_the_build() {
    unsafe {
        ipdb_reset();
        let junk = [0u8; 64];
        let p = "Loose/broken.mp3";
        assert_eq!(ipdb_add_track(p.as_ptr(), p.len(), junk.as_ptr(), junk.len(), 0), 0);
        assert_eq!(add("ok.flac", "Halcyon Drift/Low Tide Lights/01 After the Static.flac"), 1);

        let prefix = "/Music";
        assert_eq!(ipdb_build(prefix.as_ptr(), prefix.len(), 1), 1);
        assert!(ipdb_warnings() > 0);
        let w = std::str::from_utf8(std::slice::from_raw_parts(ipdb_warn_ptr(), ipdb_warn_len())).unwrap();
        assert!(w.contains("broken.mp3"));
    }
}

#[test]
fn folder_art_applies_to_tracks_without_embedded_covers() {
    unsafe {
        ipdb_reset();
        assert_eq!(add("The Quiet Hours/Northbound/CD1/01 Copper Sky.m4a",
                       "The Quiet Hours/Northbound/CD1/01 Copper Sky.m4a"), 1);
        let cover = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cover_c.jpg")).unwrap();
        let dir = "The Quiet Hours/Northbound/CD1";
        assert_eq!(ipdb_add_folder_art(dir.as_ptr(), dir.len(), cover.as_ptr(), cover.len()), 1);

        let prefix = "/Music";
        assert_eq!(ipdb_build(prefix.as_ptr(), prefix.len(), 7), 1);
        let raw = db_bytes();
        let db = Db::parse(&raw).unwrap();
        let album = db.album(0);
        assert_ne!(album.art_id, ipdb::format::NONE, "folder art should be attached");
        assert_ne!(album.colors, [0; 3], "dominant colours should be extracted");
    }
}

#[test]
fn fonts_build_through_the_abi() {
    unsafe {
        assert!(ipdb_font_count() >= 6);
        let mut name = [0u8; 64];
        let n = ipdb_font_name(0, name.as_mut_ptr(), name.len());
        assert!(n > 0);
        let name = std::str::from_utf8(&name[..n]).unwrap();
        assert!(name.ends_with(".ipfn"), "{name}");

        assert_eq!(ipdb_build_font(0), 1);
        assert!(ipdb_font_len() > 1000);
        assert_eq!(&std::slice::from_raw_parts(ipdb_font_ptr(), 4), b"IPFN");

        assert_eq!(ipdb_build_font(9999), 0);
    }
}

#[test]
fn alloc_and_free_round_trip() {
    unsafe {
        let p = ipdb_alloc(1024);
        assert!(!p.is_null());
        std::ptr::write_bytes(p, 0xAB, 1024);
        assert_eq!(*p.add(1023), 0xAB);
        ipdb_free(p, 1024);
        let _ = std::ptr::null::<c_void>();
    }
}

#[test]
fn cached_blobs_reproduce_the_same_library() {
    unsafe {
        // First pass: parse the files and keep each track's cache blob.
        ipdb_reset();
        let files = [
            "Halcyon Drift/Low Tide Lights/01 After the Static.flac",
            "Mara Vell/Paper Moons/01 Barrow Lane.mp3",
            "The Quiet Hours/Northbound/CD1/01 Copper Sky.m4a",
        ];
        let mut blobs = Vec::new();
        for f in files {
            assert_eq!(add(f, f), 1);
            blobs.push(std::slice::from_raw_parts(ipdb_blob_ptr(), ipdb_blob_len()).to_vec());
        }
        let prefix = "/Music";
        assert_eq!(ipdb_build(prefix.as_ptr(), prefix.len(), 77), 3);
        let from_files = db_bytes();
        let art_from_files = art_bytes();

        // Second pass: feed the blobs back, never touching the files.
        ipdb_reset();
        for (f, blob) in files.iter().zip(&blobs) {
            assert_eq!(ipdb_add_cached(f.as_ptr(), f.len(), blob.as_ptr(), blob.len()), 1,
                       "cached blob should be accepted for {f}");
        }
        assert_eq!(ipdb_build(prefix.as_ptr(), prefix.len(), 77), 3);

        assert_eq!(db_bytes(), from_files, "cached build must match the parsed one");
        assert_eq!(art_bytes(), art_from_files, "artwork must survive the cache");
    }
}

#[test]
fn a_corrupt_blob_is_refused_so_the_caller_can_fall_back() {
    unsafe {
        ipdb_reset();
        let p = "x.flac";
        let junk = [1u8, 2, 3, 4, 5, 6];
        assert_eq!(ipdb_add_cached(p.as_ptr(), p.len(), junk.as_ptr(), junk.len()), 0);
        assert_eq!(ipdb_track_count(), 0);
    }
}
