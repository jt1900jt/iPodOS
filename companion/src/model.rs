use crate::format::{Codec, JUMP_BUCKETS, JUMP_ROWS};

/// One audio file as read from tags and file properties. Produced by the scanner
/// (or, in the browser, by JS feeding bytes through `tags::read_bytes`).
#[derive(Clone, Debug, Default)]
pub struct TrackMeta {
    /// Path relative to the library root, '/'-separated.
    pub rel_path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub track_no: u16,
    pub disc_no: u16,
    pub year: u16,
    pub compilation: bool,
    pub audiobook: bool,
    pub duration_ms: u32,
    pub sample_rate: u32,
    pub bits: u8,
    pub channels: u8,
    pub bitrate_kbps: u16,
    pub codec: Codec,
    pub rg_track_db: Option<f32>,
    pub rg_album_db: Option<f32>,
    pub file_size: u32,
    pub mtime: u32,
    /// Index into the caller's art source list.
    pub art_source: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct PlaylistMeta {
    pub name: String,
    /// Entries as library-relative paths.
    pub entries: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TrackRec {
    pub uid: u32,
    pub path: u32,
    pub title: u32,
    pub artist_id: u32,
    pub album_id: u32,
    pub genre_id: u32,
    pub composer_id: u32,
    pub duration_ms: u32,
    pub sample_rate: u32,
    pub file_size: u32,
    pub mtime: u32,
    pub track_no: u16,
    pub disc_no: u16,
    pub year: u16,
    pub bitrate_kbps: u16,
    pub rg_track_cdb: i16,
    pub rg_album_cdb: i16,
    pub codec: u8,
    pub bits: u8,
    pub channels: u8,
    pub flags: u8,
}

#[derive(Clone, Debug, Default)]
pub struct AlbumRec {
    pub title: u32,
    pub artist_id: u32,
    pub art_id: u32,
    pub tracks_first: u32,
    pub tracks_count: u32,
    pub year: u16,
    pub flags: u8,
    pub colors: [u16; 3],
}

/// Artist, genre, composer and playlist records share this layout.
#[derive(Clone, Debug, Default)]
pub struct GroupRec {
    pub name: u32,
    pub first: u32,
    pub count: u32,
    pub extra: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    pub strings: Vec<u8>,
    pub tracks: Vec<TrackRec>,
    pub albums: Vec<AlbumRec>,
    pub artists: Vec<GroupRec>,
    pub genres: Vec<GroupRec>,
    pub composers: Vec<GroupRec>,
    pub playlists: Vec<GroupRec>,
    pub album_tracks: Vec<u32>,
    pub artist_albums: Vec<u32>,
    pub genre_artists: Vec<u32>,
    pub composer_tracks: Vec<u32>,
    pub playlist_tracks: Vec<u32>,
    pub jump: [[u32; JUMP_BUCKETS]; JUMP_ROWS],
    /// art_id -> caller's art source index.
    pub art_sources: Vec<usize>,
}
