use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use ipdb::build::{build_library, BuildOptions};
use ipdb::font;
use ipdb::format::*;
use ipdb::read::{Db, PackHeader};
use ipdb::{art, scan, write};

const DB_NAME: &str = "library.ipdb";
const ART_NAME: &str = "artwork.ipap";

#[derive(Parser)]
#[command(name = "ipdb", about = "Build and inspect iPod OS library databases")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan a music folder and write library.ipdb + artwork.ipap
    Build {
        /// Music folder (mirrors the device's music directory)
        #[arg(long)]
        source: PathBuf,
        /// Output directory, e.g. <ipod mount>/.ipodos
        #[arg(long)]
        out: PathBuf,
        /// Device path the music folder maps to
        #[arg(long, default_value = "/Music")]
        prefix: String,
        /// Override the generation number (default: max(now, previous + 1))
        #[arg(long)]
        generation: Option<u64>,
    },
    /// Print a summary of a built library
    Dump {
        /// Directory containing library.ipdb, or the file itself
        path: PathBuf,
        /// List every track
        #[arg(long)]
        tracks: bool,
        /// List albums with their tracks
        #[arg(long)]
        albums: bool,
    },
    /// Build the UI font atlases into <out>/fonts
    Fonts {
        /// Output directory, e.g. <ipod mount>/.ipodos
        #[arg(long)]
        out: PathBuf,
        /// Directory holding the Inter .ttf files (defaults to the built-in copies)
        #[arg(long)]
        ttf_dir: Option<PathBuf>,
        /// Also write a PNG preview of each face
        #[arg(long)]
        preview: bool,
    },
    /// Validate library.ipdb and artwork.ipap in a directory
    Verify { dir: PathBuf },
    /// Export one art slot as PNG (for checking rendering)
    ArtExport {
        dir: PathBuf,
        /// Class id: THMB, HEAD, LRGE or BLUR
        #[arg(long)]
        class: String,
        #[arg(long)]
        id: u32,
        #[arg(long)]
        out: PathBuf,
    },
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn previous_generation(dir: &Path) -> Option<u64> {
    let mut head = [0u8; 32];
    File::open(dir.join(DB_NAME)).ok()?.read_exact(&mut head).ok()?;
    (head[0..4] == DB_MAGIC).then(|| u64::from_le_bytes(head[24..32].try_into().unwrap()))
}

fn tmp_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.tmp"))
}

fn commit(dir: &Path, name: &str) -> Result<()> {
    let tmp = tmp_path(dir, name);
    File::open(&tmp)?.sync_all()?;
    fs::rename(&tmp, dir.join(name)).with_context(|| format!("renaming {}", tmp.display()))
}

fn cmd_build(source: &Path, out: &Path, prefix: &str, generation: Option<u64>) -> Result<()> {
    fs::create_dir_all(out)?;
    let t0 = std::time::Instant::now();
    let scanned = scan::scan(source)?;
    for w in &scanned.warnings {
        eprintln!("warning: {w}");
    }
    let mut lib = build_library(&scanned.tracks, &scanned.playlists, &BuildOptions { path_prefix: prefix.into() });
    let gen = generation.unwrap_or_else(|| {
        let n = now();
        match previous_generation(out) {
            Some(p) if p >= n => p + 1,
            _ => n,
        }
    });

    // Art first: the pack must exist before a DB that references it.
    let art_tmp = tmp_path(out, ART_NAME);
    let mut f = OpenOptions::new().create(true).write(true).truncate(true).read(true).open(&art_tmp)?;
    let sources = lib.art_sources.clone();
    let colors = art::write_pack(
        &mut f,
        sources.len(),
        gen,
        |i| scanned.art[sources[i]].load(),
        |i, e| eprintln!("warning: art {i}: {e}"),
    )?;
    drop(f);
    for a in lib.albums.iter_mut() {
        if a.art_id != NONE {
            a.colors = colors[a.art_id as usize];
        }
    }

    let bytes = write::write_library(&lib, gen, now());
    Db::parse(&bytes).context("self-check of written library failed")?;
    File::create(tmp_path(out, DB_NAME))?.write_all(&bytes)?;

    commit(out, ART_NAME)?;
    commit(out, DB_NAME)?;

    println!(
        "{} tracks, {} albums, {} artists, {} genres, {} composers, {} playlists, {} covers",
        lib.tracks.len(),
        lib.albums.len(),
        lib.artists.len(),
        lib.genres.len(),
        lib.composers.len(),
        lib.playlists.len(),
        sources.len()
    );
    println!(
        "generation {gen}; {DB_NAME} {} bytes, {ART_NAME} {} bytes; {:.2?}",
        bytes.len(),
        fs::metadata(out.join(ART_NAME))?.len(),
        t0.elapsed()
    );
    Ok(())
}

/// The four Inter weights are compiled in so the CLI works without the repo checkout.
fn builtin_ttf(name: &str) -> Option<&'static [u8]> {
    match name {
        "Inter-Regular.ttf" => Some(include_bytes!("../assets/Inter-Regular.ttf")),
        "Inter-Medium.ttf" => Some(include_bytes!("../assets/Inter-Medium.ttf")),
        "Inter-SemiBold.ttf" => Some(include_bytes!("../assets/Inter-SemiBold.ttf")),
        "Inter-Bold.ttf" => Some(include_bytes!("../assets/Inter-Bold.ttf")),
        _ => None,
    }
}

fn cmd_fonts(out: &Path, ttf_dir: Option<&Path>, preview: bool) -> Result<()> {
    let dir = out.join("fonts");
    fs::create_dir_all(&dir)?;
    for f in font::FACES {
        let ttf: Vec<u8> = match ttf_dir {
            Some(d) => fs::read(d.join(f.ttf)).with_context(|| format!("reading {}", d.join(f.ttf).display()))?,
            None => builtin_ttf(f.ttf).with_context(|| format!("no built-in copy of {}", f.ttf))?.to_vec(),
        };
        let atlas = font::build_face(&ttf, f.px, f.tracking)?;
        let path = dir.join(format!("{}.ipfn", f.name));
        fs::write(&path, &atlas)?;
        println!("{:<10} {:>3}px  {:>6} bytes  {}", f.name, f.px, atlas.len(), f.ttf);
        if preview {
            let png = dir.join(format!("{}.png", f.name));
            font_preview(&atlas, &png)?;
        }
    }
    Ok(())
}

/// Render a sample string from an atlas so the glyphs can be eyeballed without a device.
fn font_preview(atlas: &[u8], out: &Path) -> Result<()> {
    const SAMPLE: &str = "Halcyon Drift \u{2022} Low Tide Lights 3:00 \u{00C5}\u{00D8}\u{2014}";
    let g = font::Atlas::parse(atlas)?;
    let (w, h) = (g.measure(SAMPLE) as u32 + 8, g.line_height() as u32 + 8);
    let mut img = image::GrayImage::new(w.max(1), h.max(1));
    g.draw(SAMPLE, 4, 4 + g.ascent() as i32, |x, y, a| {
        if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
            let p = img.get_pixel_mut(x as u32, y as u32);
            p[0] = p[0].max(a);
        }
    });
    img.save(out)?;
    Ok(())
}

fn db_path(p: &Path) -> PathBuf {
    if p.is_dir() {
        p.join(DB_NAME)
    } else {
        p.to_path_buf()
    }
}

fn fmt_ms(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn cmd_dump(path: &Path, tracks: bool, albums: bool) -> Result<()> {
    let buf = fs::read(db_path(path))?;
    let db = Db::parse(&buf)?;
    println!("generation {}  created {}  minor {}", db.generation, db.created, db.version_minor);
    let counts = [
        db.track_count(),
        db.album_count(),
        db.group_count(sec::ARTS),
        db.group_count(sec::GENR),
        db.group_count(sec::COMP),
    ];
    println!(
        "tracks {}  albums {}  artists {}  genres {}  composers {}  playlists {}",
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        db.group_count(sec::PLST)
    );
    let letters: Vec<char> = ('A'..='Z').chain(std::iter::once('#')).collect();
    for (row, name) in ["songs", "albums", "artists", "genres", "composers"].iter().enumerate() {
        let j = db.jump_row(row);
        let used: Vec<String> = (0..JUMP_BUCKETS)
            .filter(|&b| {
                let end = if b + 1 < JUMP_BUCKETS { j[b + 1] } else { counts[row] as u32 };
                j[b] < end
            })
            .map(|b| format!("{}@{}", letters[b], j[b]))
            .collect();
        println!("jump {name:<9} {}", used.join(" "));
    }
    if albums {
        for i in 0..db.album_count() {
            let a = db.album(i);
            let artist = db.group(sec::ARTS, a.artist_id as usize);
            println!(
                "[{i}] {} — {} ({}) art={} colors={:04x},{:04x},{:04x}{}",
                db.string(a.title),
                db.string(artist.name),
                a.year,
                if a.art_id == NONE { "-".into() } else { a.art_id.to_string() },
                a.colors[0],
                a.colors[1],
                a.colors[2],
                if a.flags & album_flags::COMPILATION != 0 { " [comp]" } else { "" }
            );
            for k in a.tracks_first..a.tracks_first + a.tracks_count {
                let t = db.track(db.index(sec::IALB, k as usize) as usize);
                println!(
                    "    {}-{:02} {} — {}  {}  {}",
                    t.disc_no,
                    t.track_no,
                    db.string(t.title),
                    db.string(db.group(sec::ARTS, t.artist_id as usize).name),
                    fmt_ms(t.duration_ms),
                    Codec::name(t.codec)
                );
            }
        }
    }
    if tracks {
        for i in 0..db.track_count() {
            let t = db.track(i);
            let rg = if t.flags & track_flags::RG_TRACK != 0 {
                format!(" rg={:+.2}dB", t.rg_track_cdb as f32 / 100.0)
            } else {
                String::new()
            };
            println!(
                "[{i}] {} — {}  {}  {} {}Hz/{}bit/{}ch {}kbps{}  uid={:08x}  {}",
                db.string(t.title),
                db.string(db.group(sec::ARTS, t.artist_id as usize).name),
                fmt_ms(t.duration_ms),
                Codec::name(t.codec),
                t.sample_rate,
                t.bits,
                t.channels,
                t.bitrate_kbps,
                rg,
                t.uid,
                db.string(t.path)
            );
        }
    }
    for i in 0..db.group_count(sec::PLST) {
        let p = db.group(sec::PLST, i);
        println!("playlist {} ({} tracks)", db.string(p.name), p.count);
    }
    Ok(())
}

fn read_pack_header(dir: &Path) -> Result<PackHeader> {
    let path = dir.join(ART_NAME);
    let len = fs::metadata(&path)?.len();
    let mut head = vec![0u8; 4096.min(len as usize)];
    File::open(&path)?.read_exact(&mut head)?;
    PackHeader::parse(&head, len)
}

fn cmd_verify(dir: &Path) -> Result<()> {
    let buf = fs::read(dir.join(DB_NAME))?;
    let db = Db::parse(&buf).context(DB_NAME)?;
    let pack = read_pack_header(dir).context(ART_NAME)?;
    if pack.generation != db.generation {
        bail!("generation mismatch: db {} vs art pack {}", db.generation, pack.generation);
    }
    for i in 0..db.album_count() {
        let a = db.album(i);
        if a.art_id != NONE && a.art_id >= pack.art_count {
            bail!("album {i} references art {} but pack has {}", a.art_id, pack.art_count);
        }
    }
    for c in &ART_CLASSES {
        if pack.class(&c.id).is_none() {
            bail!("art pack missing class {}", fourcc_str(&c.id));
        }
    }
    println!(
        "ok: generation {}, {} tracks, {} albums, {} covers",
        db.generation,
        db.track_count(),
        db.album_count(),
        pack.art_count
    );
    Ok(())
}

fn cmd_art_export(dir: &Path, class: &str, id: u32, out: &Path) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    let pack = read_pack_header(dir)?;
    let cid: [u8; 4] = class.as_bytes().try_into().context("class id must be 4 characters")?;
    let c = pack.class(&cid).with_context(|| format!("no class {class}"))?;
    if id >= pack.art_count {
        bail!("art id {id} out of range (pack has {})", pack.art_count);
    }
    let slot = c.width as u64 * c.height as u64 * 2;
    let mut f = File::open(dir.join(ART_NAME))?;
    f.seek(SeekFrom::Start(c.offset + id as u64 * slot))?;
    let mut px = vec![0u8; slot as usize];
    f.read_exact(&mut px)?;
    let mut img = image::RgbImage::new(c.width, c.height);
    for (i, p) in img.pixels_mut().enumerate() {
        let v = u16::from_le_bytes([px[i * 2], px[i * 2 + 1]]);
        let (r, g, b) = ((v >> 11) & 31, (v >> 5) & 63, v & 31);
        *p = image::Rgb([(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]);
    }
    img.save(out)?;
    println!("wrote {}x{} {}", c.width, c.height, out.display());
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Build { source, out, prefix, generation } => cmd_build(&source, &out, &prefix, generation),
        Cmd::Dump { path, tracks, albums } => cmd_dump(&path, tracks, albums),
        Cmd::Fonts { out, ttf_dir, preview } => cmd_fonts(&out, ttf_dir.as_deref(), preview),
        Cmd::Verify { dir } => cmd_verify(&dir),
        Cmd::ArtExport { dir, class, id, out } => cmd_art_export(&dir, &class, id, &out),
    }
}
