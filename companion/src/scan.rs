//! Walks a library folder: reads tags, finds cover art (embedded or folder image),
//! and parses .m3u/.m3u8 playlists.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::model::{PlaylistMeta, TrackMeta};
use crate::tags::{self, AUDIO_EXTENSIONS};

const FOLDER_ART: &[&str] = &[
    "cover.jpg", "cover.jpeg", "cover.png", "folder.jpg", "folder.jpeg", "folder.png", "front.jpg", "front.png",
    "album.jpg", "album.png",
];

#[derive(Clone, Debug)]
pub enum ArtOrigin {
    Embedded(PathBuf),
    File(PathBuf),
}

impl ArtOrigin {
    pub fn load(&self) -> Result<Vec<u8>> {
        match self {
            ArtOrigin::File(p) => fs::read(p).with_context(|| format!("reading {}", p.display())),
            ArtOrigin::Embedded(p) => tags::read_path(p)?
                .picture
                .with_context(|| format!("embedded art vanished from {}", p.display())),
        }
    }
}

#[derive(Default)]
pub struct ScanResult {
    pub tracks: Vec<TrackMeta>,
    pub playlists: Vec<PlaylistMeta>,
    pub art: Vec<ArtOrigin>,
    pub warnings: Vec<String>,
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn rel_string(root: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect::<Option<_>>()?;
    Some(parts.join("/"))
}

/// Lexically normalize a path (resolve `.` and `..`) without touching the filesystem.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn scan(root: &Path) -> Result<ScanResult> {
    let root = fs::canonicalize(root).with_context(|| format!("library root {}", root.display()))?;
    let mut res = ScanResult::default();
    let mut art_by_hash: HashMap<[u8; 20], usize> = HashMap::new();
    let mut folder_art: HashMap<PathBuf, Option<usize>> = HashMap::new();
    let mut playlist_files = Vec::new();

    let walker = WalkDir::new(&root).follow_links(false).sort_by_file_name().into_iter().filter_entry(|e| {
        e.depth() == 0 || !e.file_name().to_str().map(is_hidden).unwrap_or(true)
    });
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                res.warnings.push(format!("walk: {e}"));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "m3u" || ext == "m3u8" {
            playlist_files.push(path.to_path_buf());
            continue;
        }
        if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let Some(rel) = rel_string(&root, path) else {
            res.warnings.push(format!("skipping non-UTF-8 path {}", path.display()));
            continue;
        };
        let read = match tags::read_path(path) {
            Ok(r) => r,
            Err(e) => {
                res.warnings.push(format!("{rel}: {e}"));
                continue;
            }
        };
        let mut meta = read.meta;
        meta.rel_path = rel;
        if let Ok(md) = entry.metadata() {
            meta.file_size = md.len().min(u32::MAX as u64) as u32;
            meta.mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs().min(u32::MAX as u64) as u32)
                .unwrap_or(0);
        }

        meta.art_source = match read.picture {
            Some(bytes) => {
                let h = sha1_smol::Sha1::from(&bytes).digest().bytes();
                Some(*art_by_hash.entry(h).or_insert_with(|| {
                    res.art.push(ArtOrigin::Embedded(path.to_path_buf()));
                    res.art.len() - 1
                }))
            }
            None => {
                let dir = path.parent().unwrap_or(&root).to_path_buf();
                *folder_art.entry(dir.clone()).or_insert_with(|| {
                    let found = find_folder_art(&dir)?;
                    let bytes = fs::read(&found).ok()?;
                    let h = sha1_smol::Sha1::from(&bytes).digest().bytes();
                    Some(*art_by_hash.entry(h).or_insert_with(|| {
                        res.art.push(ArtOrigin::File(found));
                        res.art.len() - 1
                    }))
                })
            }
        };
        res.tracks.push(meta);
    }

    for pl in playlist_files {
        match read_playlist(&root, &pl) {
            Ok(p) => res.playlists.push(p),
            Err(e) => res.warnings.push(format!("{}: {e}", pl.display())),
        }
    }
    Ok(res)
}

fn find_folder_art(dir: &Path) -> Option<PathBuf> {
    let entries: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| Some((e.file_name().to_str()?.to_ascii_lowercase(), e.path())))
        .collect();
    FOLDER_ART
        .iter()
        .find_map(|want| entries.iter().find(|(n, _)| n == want).map(|(_, p)| p.clone()))
}

fn read_playlist(root: &Path, path: &Path) -> Result<PlaylistMeta> {
    let raw = fs::read(path)?;
    let text = String::from_utf8_lossy(&raw);
    let base = path.parent().unwrap_or(root);
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = line.replace('\\', "/");
        let p = Path::new(&entry);
        let abs = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
        if let Some(rel) = rel_string(root, &normalize(&abs)) {
            entries.push(rel);
        }
    }
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Playlist").to_string();
    Ok(PlaylistMeta { name, entries })
}
