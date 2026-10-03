//! C ABI over the library builder, so the browser companion can build the database and
//! art pack itself rather than shelling out to the CLI.
//!
//! Deliberately not wasm-bindgen: a plain `extern "C"` surface compiles with nothing but
//! `cargo build --target wasm32-unknown-unknown` and is loadable from a page with no
//! bundler. The caller allocates with `ipdb_alloc`, fills the buffer, calls the function,
//! then reads results back through the pointer/length accessors.
//!
//! Usage:
//!   ipdb_reset()
//!   for each file: ipdb_add_track(path, bytes)   // tags parsed here
//!   for each playlist: ipdb_add_playlist(name, entries separated by '\n')
//!   ipdb_build(prefix, generation)
//!   ipdb_db_ptr()/ipdb_db_len(), ipdb_art_ptr()/ipdb_art_len()
//!
//! The same entry points are exercised natively by tests/wasm_api.rs, so the logic is
//! covered even though the wasm packaging itself is built on the developer's machine.

use std::cell::RefCell;
use std::io::Cursor;

use crate::build::{build_library, BuildOptions};
use crate::format::NONE;
use crate::model::{PlaylistMeta, TrackMeta};
use crate::{art, font, write};

#[derive(Default)]
struct State {
    tracks: Vec<TrackMeta>,
    playlists: Vec<PlaylistMeta>,
    /// Embedded or sidecar art, indexed by `TrackMeta::art_source`.
    art_blobs: Vec<Vec<u8>>,
    db: Vec<u8>,
    art: Vec<u8>,
    font: Vec<u8>,
    warnings: Vec<String>,
    warn_buf: Vec<u8>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Allocates `len` bytes for the caller to write into. Freed by `ipdb_free`.
#[no_mangle]
pub extern "C" fn ipdb_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` must come from `ipdb_alloc` with the same `len`.
#[no_mangle]
pub unsafe extern "C" fn ipdb_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len));
    }
}

unsafe fn slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

unsafe fn text(ptr: *const u8, len: usize) -> String {
    String::from_utf8_lossy(slice(ptr, len)).into_owned()
}

#[no_mangle]
pub extern "C" fn ipdb_reset() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}

/// Reads tags from one file. `path` is library-relative with '/' separators.
/// Returns 1 on success, 0 if the file could not be parsed (recorded as a warning).
///
/// # Safety
/// Pointers must be valid for their lengths.
#[no_mangle]
pub unsafe extern "C" fn ipdb_add_track(
    path_ptr: *const u8,
    path_len: usize,
    data_ptr: *const u8,
    data_len: usize,
    mtime: u32,
) -> i32 {
    let rel = text(path_ptr, path_len);
    let data = slice(data_ptr, data_len);
    let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();

    STATE.with(|s| {
        let st = &mut *s.borrow_mut();
        match crate::tags::read_bytes(data, &ext) {
            Ok(read) => {
                let mut meta = read.meta;
                meta.rel_path = rel;
                meta.file_size = data.len().min(u32::MAX as usize) as u32;
                meta.mtime = mtime;
                meta.art_source = read.picture.map(|pic| {
                    // Deduplicate by content: a 500 KB cover repeated across an album
                    // would otherwise be rendered and stored once per track.
                    let digest = sha1_smol::Sha1::from(&pic).digest().bytes();
                    if let Some(i) = st.art_blobs.iter().position(|b| {
                        sha1_smol::Sha1::from(b).digest().bytes() == digest
                    }) {
                        i
                    } else {
                        st.art_blobs.push(pic);
                        st.art_blobs.len() - 1
                    }
                });
                st.tracks.push(meta);
                1
            }
            Err(e) => {
                st.warnings.push(format!("{rel}: {e}"));
                0
            }
        }
    })
}

/// Attaches cover art to every track already added whose path starts with `dir`.
/// Used for folder art (`cover.jpg`), which has no owning track.
///
/// # Safety
/// Pointers must be valid for their lengths.
#[no_mangle]
pub unsafe extern "C" fn ipdb_add_folder_art(
    dir_ptr: *const u8,
    dir_len: usize,
    data_ptr: *const u8,
    data_len: usize,
) -> i32 {
    let dir = text(dir_ptr, dir_len);
    let data = slice(data_ptr, data_len).to_vec();
    STATE.with(|s| {
        let st = &mut *s.borrow_mut();
        let idx = st.art_blobs.len();
        let mut used = false;
        for t in st.tracks.iter_mut() {
            if t.art_source.is_some() {
                continue; // embedded art wins
            }
            let parent = t.rel_path.rfind('/').map(|i| &t.rel_path[..i]).unwrap_or("");
            if parent == dir {
                t.art_source = Some(idx);
                used = true;
            }
        }
        if used {
            st.art_blobs.push(data);
            1
        } else {
            0
        }
    })
}

/// Adds a playlist. `entries` is library-relative paths separated by newlines.
///
/// # Safety
/// Pointers must be valid for their lengths.
#[no_mangle]
pub unsafe extern "C" fn ipdb_add_playlist(
    name_ptr: *const u8,
    name_len: usize,
    entries_ptr: *const u8,
    entries_len: usize,
) {
    let name = text(name_ptr, name_len);
    let entries = text(entries_ptr, entries_len)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    STATE.with(|s| s.borrow_mut().playlists.push(PlaylistMeta { name, entries }));
}

#[no_mangle]
pub extern "C" fn ipdb_track_count() -> u32 {
    STATE.with(|s| s.borrow().tracks.len() as u32)
}

/// Builds the database and art pack. Returns the track count, or -1 on failure.
///
/// # Safety
/// Pointers must be valid for their lengths.
#[no_mangle]
pub unsafe extern "C" fn ipdb_build(prefix_ptr: *const u8, prefix_len: usize, generation: u64) -> i32 {
    let prefix = text(prefix_ptr, prefix_len);
    STATE.with(|s| {
        let st = &mut *s.borrow_mut();
        let opts = BuildOptions { path_prefix: if prefix.is_empty() { "/Music".into() } else { prefix } };
        let mut lib = build_library(&st.tracks, &st.playlists, &opts);

        let sources = lib.art_sources.clone();
        let mut pack = Cursor::new(Vec::new());
        let blobs = &st.art_blobs;
        let mut warnings = Vec::new();
        let colors = match art::write_pack(
            &mut pack,
            sources.len(),
            generation,
            |i| Ok(blobs[sources[i]].clone()),
            |i, e| warnings.push(format!("art {i}: {e}")),
        ) {
            Ok(c) => c,
            Err(e) => {
                st.warnings.push(format!("art pack: {e}"));
                return -1;
            }
        };
        st.warnings.extend(warnings);
        for a in lib.albums.iter_mut() {
            if a.art_id != NONE {
                a.colors = colors[a.art_id as usize];
            }
        }

        st.db = write::write_library(&lib, generation, generation as u32 as u64);
        st.art = pack.into_inner();
        lib.tracks.len() as i32
    })
}

/// Builds one font atlas by index into `font::FACES`; read back with the font accessors.
/// Returns 1 on success, 0 when the index is out of range.
#[no_mangle]
pub extern "C" fn ipdb_build_font(index: u32) -> i32 {
    let Some(spec) = font::FACES.get(index as usize) else {
        return 0;
    };
    let ttf: &[u8] = match spec.ttf {
        "Inter-Regular.ttf" => include_bytes!("../assets/Inter-Regular.ttf"),
        "Inter-Medium.ttf" => include_bytes!("../assets/Inter-Medium.ttf"),
        "Inter-SemiBold.ttf" => include_bytes!("../assets/Inter-SemiBold.ttf"),
        _ => include_bytes!("../assets/Inter-Bold.ttf"),
    };
    match font::build_face(ttf, spec.px, spec.tracking) {
        Ok(data) => STATE.with(|s| {
            s.borrow_mut().font = data;
            1
        }),
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn ipdb_font_count() -> u32 {
    font::FACES.len() as u32
}

/// Writes the name of face `index` into `out`, returning the byte length (0 if absent).
///
/// # Safety
/// `out` must be valid for `cap` bytes.
#[no_mangle]
pub unsafe extern "C" fn ipdb_font_name(index: u32, out: *mut u8, cap: usize) -> usize {
    let Some(spec) = font::FACES.get(index as usize) else {
        return 0;
    };
    let name = format!("{}.ipfn", spec.name);
    let bytes = name.as_bytes();
    if bytes.len() > cap || out.is_null() {
        return 0;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
    bytes.len()
}

macro_rules! buffer_accessors {
    ($ptr_fn:ident, $len_fn:ident, $field:ident) => {
        #[no_mangle]
        pub extern "C" fn $ptr_fn() -> *const u8 {
            STATE.with(|s| s.borrow().$field.as_ptr())
        }
        #[no_mangle]
        pub extern "C" fn $len_fn() -> usize {
            STATE.with(|s| s.borrow().$field.len())
        }
    };
}

buffer_accessors!(ipdb_db_ptr, ipdb_db_len, db);
buffer_accessors!(ipdb_art_ptr, ipdb_art_len, art);
buffer_accessors!(ipdb_font_ptr, ipdb_font_len, font);

/// Joins the warnings with newlines and exposes them through the warning accessors.
#[no_mangle]
pub extern "C" fn ipdb_warnings() -> usize {
    STATE.with(|s| {
        let st = &mut *s.borrow_mut();
        st.warn_buf = st.warnings.join("\n").into_bytes();
        st.warn_buf.len()
    })
}

buffer_accessors!(ipdb_warn_ptr, ipdb_warn_len, warn_buf);
