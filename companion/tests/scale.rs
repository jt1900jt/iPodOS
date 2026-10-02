//! Synthetic large library: sizes and timings for the RAM budget.
//! cargo test --release --test scale -- --ignored --nocapture

use std::time::Instant;

use ipdb::build::{build_library, BuildOptions};
use ipdb::format::Codec;
use ipdb::model::{PlaylistMeta, TrackMeta};
use ipdb::read::Db;
use ipdb::write;

fn synth(n: usize) -> Vec<TrackMeta> {
    let words = ["Glass", "Northern", "Paper", "Static", "Copper", "Neon", "Quiet", "Harbor", "Velvet", "Signal",
                 "Moon", "River", "Ember", "Hollow", "Sky", "Drift"];
    let w = |i: usize, k: usize| format!("{} {}", words[(i * 7 + k) % 16], words[(i / 16 + k * 3) % 16]);
    (0..n)
        .map(|i| {
            let artist = i / 120; // ~120 tracks per artist
            let album = i / 12; // 12 tracks per album
            TrackMeta {
                rel_path: format!("Artist {artist:04}/Album {album:05}/{:02} Track {i}.flac", i % 12 + 1),
                title: Some(format!("{} {i}", w(i, 0))),
                artist: Some(format!("{} Band {artist}", w(artist, 1))),
                album: Some(format!("{} {album}", w(album, 2))),
                genre: Some(words[artist % 16].into()),
                composer: (i % 5 == 0).then(|| format!("Composer {}", i % 300)),
                track_no: (i % 12 + 1) as u16,
                year: 1970 + (album % 50) as u16,
                duration_ms: 200_000,
                sample_rate: 44_100,
                bits: 16,
                channels: 2,
                codec: Codec::Flac,
                art_source: Some(album),
                ..Default::default()
            }
        })
        .collect()
}

#[test]
#[ignore]
fn scale_report() {
    for n in [20_000usize, 50_000, 100_000] {
        let tracks = synth(n);
        let pls: Vec<PlaylistMeta> = (0..20)
            .map(|p| PlaylistMeta {
                name: format!("Mix {p}"),
                entries: tracks.iter().skip(p).step_by(97).map(|t| t.rel_path.clone()).collect(),
            })
            .collect();
        let t0 = Instant::now();
        let lib = build_library(&tracks, &pls, &BuildOptions::default());
        let t_build = t0.elapsed();
        let bytes = write::write_library(&lib, 1, 0);
        let t1 = Instant::now();
        let db = Db::parse(&bytes).unwrap();
        let t_parse = t1.elapsed();
        assert_eq!(db.track_count(), n);
        println!(
            "{n:>7} tracks: {:>6.2} MB ({:.0} B/track), strings {:.2} MB, {} albums, {} artists; build {:?}, validate {:?}",
            bytes.len() as f64 / 1e6,
            bytes.len() as f64 / n as f64,
            lib.strings.len() as f64 / 1e6,
            lib.albums.len(),
            lib.artists.len(),
            t_build,
            t_parse
        );
        if n == 50_000 {
            std::fs::create_dir_all("target/scale-library").unwrap();
            std::fs::write("target/scale-library/library.ipdb", &bytes).unwrap();
        }
    }
}
