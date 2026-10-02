//! On-disk constants shared by the writer and reader. See docs/ipdb-format.md.

pub const NONE: u32 = u32::MAX;

pub const DB_MAGIC: [u8; 4] = *b"IPDB";
pub const ART_MAGIC: [u8; 4] = *b"IPAP";
pub const VERSION_MAJOR: u16 = 1;
pub const VERSION_MINOR: u16 = 0;
pub const HEADER_SIZE: u32 = 64;
pub const SECTION_ENTRY_SIZE: u32 = 16;
pub const SECTION_ALIGN: usize = 16;

pub const TRACK_SIZE: usize = 64;
pub const ALBUM_SIZE: usize = 32;
pub const GROUP_SIZE: usize = 16;

pub const JUMP_BUCKETS: usize = 27;
pub const JUMP_ROWS: usize = 5;
pub const JUMP_SONGS: usize = 0;
pub const JUMP_ALBUMS: usize = 1;
pub const JUMP_ARTISTS: usize = 2;
pub const JUMP_GENRES: usize = 3;
pub const JUMP_COMPOSERS: usize = 4;

/// Section IDs, in the order the writer emits them.
pub mod sec {
    pub const STRS: [u8; 4] = *b"STRS";
    pub const TRKS: [u8; 4] = *b"TRKS";
    pub const ALBM: [u8; 4] = *b"ALBM";
    pub const ARTS: [u8; 4] = *b"ARTS";
    pub const GENR: [u8; 4] = *b"GENR";
    pub const COMP: [u8; 4] = *b"COMP";
    pub const PLST: [u8; 4] = *b"PLST";
    pub const IALB: [u8; 4] = *b"IALB";
    pub const IART: [u8; 4] = *b"IART";
    pub const IGEN: [u8; 4] = *b"IGEN";
    pub const ICMP: [u8; 4] = *b"ICMP";
    pub const IPLS: [u8; 4] = *b"IPLS";
    pub const JUMP: [u8; 4] = *b"JUMP";

    pub const ALL: [[u8; 4]; 13] = [
        STRS, TRKS, ALBM, ARTS, GENR, COMP, PLST, IALB, IART, IGEN, ICMP, IPLS, JUMP,
    ];
}

pub mod track_flags {
    pub const RG_TRACK: u8 = 1 << 0;
    pub const RG_ALBUM: u8 = 1 << 1;
    pub const COMPILATION: u8 = 1 << 2;
    pub const LOSSLESS: u8 = 1 << 3;
    pub const AUDIOBOOK: u8 = 1 << 4;
}

pub mod album_flags {
    pub const COMPILATION: u8 = 1 << 0;
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Codec {
    #[default]
    Unknown = 0,
    Mp3 = 1,
    Aac = 2,
    Alac = 3,
    Flac = 4,
    Vorbis = 5,
    Opus = 6,
    Wav = 7,
    Aiff = 8,
    WavPack = 9,
    Ape = 10,
    Musepack = 11,
}

impl Codec {
    pub fn is_lossless(self) -> bool {
        matches!(
            self,
            Codec::Alac | Codec::Flac | Codec::Wav | Codec::Aiff | Codec::WavPack | Codec::Ape
        )
    }

    pub fn name(code: u8) -> &'static str {
        match code {
            1 => "MP3",
            2 => "AAC",
            3 => "ALAC",
            4 => "FLAC",
            5 => "Vorbis",
            6 => "Opus",
            7 => "WAV",
            8 => "AIFF",
            9 => "WavPack",
            10 => "APE",
            11 => "Musepack",
            _ => "unknown",
        }
    }
}

/// Art pack classes. Readers look classes up by id, so sizes can change freely.
#[derive(Clone, Copy, Debug)]
pub struct ArtClass {
    pub id: [u8; 4],
    pub width: u32,
    pub height: u32,
    pub blur: bool,
}

impl ArtClass {
    pub fn slot_size(&self) -> u64 {
        self.width as u64 * self.height as u64 * 2
    }
}

pub const ART_CLASSES: [ArtClass; 4] = [
    ArtClass { id: *b"THMB", width: 28, height: 28, blur: false },
    ArtClass { id: *b"HEAD", width: 52, height: 52, blur: false },
    ArtClass { id: *b"LRGE", width: 116, height: 116, blur: false },
    ArtClass { id: *b"BLUR", width: 80, height: 60, blur: true },
];

pub const ART_CLASS_ENTRY_SIZE: u32 = 16;
pub const ART_DATA_ALIGN: u64 = 4096;
pub const ART_FALLBACK_PIXEL: u16 = 0x2104;

pub fn fourcc_str(id: &[u8; 4]) -> String {
    String::from_utf8_lossy(id).into_owned()
}

pub fn align_up(v: usize, a: usize) -> usize {
    (v + a - 1) / a * a
}
