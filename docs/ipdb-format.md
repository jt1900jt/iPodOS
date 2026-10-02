# Library database and art pack formats (v1.0)

Two files live on the device under `/.ipodos/`:

- `library.ipdb` — the music library. Small (MBs), loaded fully into RAM, validated once at load.
- `artwork.ipap` — pre-rendered album art. Large, never loaded whole; slots are read on demand.

Both are produced by the companion (`ipdb build`). The device treats them as read-only and replaces them only by atomic rename. Mutable state (play counts, ratings, resume positions) lives in a separate journal, not in these files.

## Conventions

- All integers little-endian. Records and sections are designed for direct casting on a little-endian host (ARM926, x86, wasm32), with no unaligned fields.
- Every section starts on a 16-byte boundary. The loader must place the file in a 4-byte-aligned buffer.
- `NONE` = `0xFFFFFFFF`, used for absent IDs.
- A "string offset" is a byte offset into the `STRS` section. Strings are UTF-8, NUL-terminated, contain no interior NUL, and offset 0 is the empty string.
- An "ID" is a zero-based index into a record table.
- FourCC values are four ASCII bytes stored in file order, read as a little-endian `u32`. For example, `"TRKS"` reads as `0x534B5254`.

## library.ipdb

### Header (64 bytes)

| Off | Type | Field |
|---|---|---|
| 0 | char[4] | magic `IPDB` |
| 4 | u16 | version_major (1; readers reject any other value) |
| 6 | u16 | version_minor (0; readers ignore higher values) |
| 8 | u32 | header_size (64) |
| 12 | u32 | section_count |
| 16 | u32 | section_table_offset (64) |
| 20 | u32 | file_size |
| 24 | u64 | generation (shared with the art pack) |
| 32 | u64 | created (unix seconds) |
| 40 | u32 | crc32 (IEEE) of bytes `[header_size, file_size)` |
| 44 | u32 | track_count (same value as the TRKS count) |
| 48 | u32 | flags (0) |
| 52 | u8[12] | reserved, zero |

### Section table

`section_count` entries of 16 bytes each, at `section_table_offset`:

| Off | Type | Field |
|---|---|---|
| 0 | u32 | id (FourCC) |
| 4 | u32 | offset (16-aligned) |
| 8 | u32 | size in bytes |
| 12 | u32 | count (records or entries) |

Each section must lie within `file_size`. For record sections, `size == count * record_size`. Unknown section IDs are ignored. Every v1.0 section listed below is required.

### Sections

| ID | Record | Contents |
|---|---|---|
| `STRS` | bytes | String pool. `count` = 0. Must begin and end with a NUL byte. |
| `TRKS` | Track, 64 B | All tracks, in Songs-list order |
| `ALBM` | Album, 32 B | All albums, in Albums-list order |
| `ARTS` | Group, 16 B | All artists, in Artists-list order |
| `GENR` | Group, 16 B | Genres, in list order |
| `COMP` | Group, 16 B | Composers, in list order |
| `PLST` | Group, 16 B | Playlists, in list order |
| `IALB` | u32 | Track IDs per album, in play order (disc, track) |
| `IART` | u32 | Album IDs per artist (by year, then title) |
| `IGEN` | u32 | Artist IDs per genre (in artist order) |
| `ICMP` | u32 | Track IDs per composer (in title order) |
| `IPLS` | u32 | Track IDs per playlist (in file order) |
| `JUMP` | u32[27] | Letter-jump rows, `count` = 5 |

Each table is stored already sorted, so a table's natural ID order is its display order. The "all songs", "all albums", and "all artists" lists need no index: list row *i* is ID *i*. Child lists use CSR (compressed sparse row) layout. A parent record holds `first` and `count`, and its children are the index-section slice `[first, first + count)`.

### Track (64 bytes)

| Off | Type | Field |
|---|---|---|
| 0 | u32 | uid (stable across rebuilds while the path is unchanged; journal key) |
| 4 | u32 | path (string; absolute device path) |
| 8 | u32 | title (string) |
| 12 | u32 | artist_id |
| 16 | u32 | album_id |
| 20 | u32 | genre_id or NONE |
| 24 | u32 | composer_id or NONE |
| 28 | u32 | duration_ms |
| 32 | u32 | sample_rate (Hz) |
| 36 | u32 | file_size (bytes) |
| 40 | u32 | mtime (unix seconds) |
| 44 | u16 | track_no (0 = unknown) |
| 46 | u16 | disc_no (0 = unknown) |
| 48 | u16 | year (0 = unknown) |
| 50 | u16 | bitrate_kbps |
| 52 | i16 | ReplayGain track gain, in hundredths of a dB |
| 54 | i16 | ReplayGain album gain, in hundredths of a dB |
| 56 | u8 | codec (see below) |
| 57 | u8 | bits_per_sample (0 for lossy codecs) |
| 58 | u8 | channels |
| 59 | u8 | flags (see below) |
| 60 | u32 | reserved |

Codec values: 0 unknown, 1 MP3, 2 AAC, 3 ALAC, 4 FLAC, 5 Vorbis, 6 Opus, 7 WAV, 8 AIFF, 9 WavPack, 10 APE, 11 Musepack.

Track flags:

| Bit | Meaning |
|---|---|
| 0 | track gain is valid |
| 1 | album gain is valid |
| 2 | track is on a compilation |
| 3 | codec is lossless |
| 4 | audiobook |

### Album (32 bytes)

| Off | Type | Field |
|---|---|---|
| 0 | u32 | title (string) |
| 4 | u32 | artist_id (album artist) |
| 8 | u32 | art_id (art pack slot) or NONE |
| 12 | u32 | tracks_first (into `IALB`) |
| 16 | u32 | tracks_count |
| 20 | u16 | year |
| 22 | u8 | flags (bit 0 = compilation) |
| 23 | u8 | reserved |
| 24 | u16[3] | dominant colors (RGB565), most prominent first; zero if there is no art |
| 30 | u16 | reserved |

### Group (16 bytes)

The four group tables share one layout:

| Off | Artist (ARTS) | Genre (GENR) | Composer (COMP) | Playlist (PLST) |
|---|---|---|---|---|
| 0 | name | name | name | name |
| 4 | albums_first (IART) | artists_first (IGEN) | tracks_first (ICMP) | tracks_first (IPLS) |
| 8 | albums_count | artists_count | tracks_count | tracks_count |
| 12 | track_count | track_count | reserved | reserved |

An artist's album list includes every album where the artist is the album artist or performs on at least one track. When browsing Artist → Album, the shell filters the album's tracks by `artist_id`.

### JUMP

`JUMP` holds five rows of 27 `u32` values, one row per list: songs (TRKS), albums (ALBM), artists (ARTS), genres (GENR), and composers (COMP), in that order. Buckets 0–25 are A–Z, and bucket 26 is `#`, which covers digits, symbols, and anything else. Bucket 26 sorts last, as on stock firmware.

`row[b]` is the index of the first entry whose bucket is ≥ b, or `count` if there is none. Bucket *b* therefore spans `[row[b], row[b+1])`, with `row[27]` taken as `count`.

### Sort keys

The sort keys are computed by the companion and never stored. They are listed here so that other builders produce the same order.

1. Transliterate the string to ASCII (deunicode), lowercase it, and trim it.
2. Strip a leading `the `, `a `, or `an `, but only if non-empty text remains.
3. Strip leading characters that are not ASCII letters or digits.
4. Bucket: `a`–`z` map to 0–25. Anything else, including an empty key, maps to 26.
5. Order by `(bucket, key)`, with tie-breakers specific to each table.

### Device-side validation (required before use)

1. Magic, major version, and `header_size`. The section table and every section must lie within `file_size`, and `file_size` must not exceed the buffer length.
2. Each section's size matches its count multiplied by its record size. Sections are 16-aligned.
3. The CRC32 matches.
4. `STRS` starts and ends with NUL, and every string offset is less than the size of `STRS`.
5. Every ID is in range or equals NONE where NONE is allowed. Every `first + count` lies within its index section, without overflow. Every index entry is in range.

After validation, all accessors are bounds-safe without further checks.

## artwork.ipap

### Header (64 bytes)

| Off | Type | Field |
|---|---|---|
| 0 | char[4] | magic `IPAP` |
| 4 | u16 | version_major (1) |
| 6 | u16 | version_minor (0) |
| 8 | u32 | header_size (64) |
| 12 | u32 | class_count |
| 16 | u32 | art_count |
| 20 | u32 | data_offset (first byte of slot data) |
| 24 | u64 | generation (must equal `library.ipdb`'s generation, or the pack is ignored) |
| 32 | u64 | file_size |
| 40 | u32 | crc32 of the header (with this field zeroed) plus the class table |
| 44 | u8[20] | reserved, zero |

The CRC deliberately does not cover the slot data. Checking hundreds of MB at boot is too slow, and a corrupt slot only produces a bad image.

### Class table

`class_count` entries of 16 bytes each, immediately after the header:

| Off | Type | Field |
|---|---|---|
| 0 | u32 | class id (FourCC) |
| 4 | u16 | width |
| 6 | u16 | height |
| 8 | u64 | file offset of this class's slot 0 |

Slot size is `width * height * 2`. Classes are stored contiguously, one class after another. Within a class, slots are in `art_id` order. Slot address = `class.offset + art_id * width * height * 2`.

Neighbouring list rows have neighbouring album IDs, so a screenful of thumbnails is one sequential read.

| Class | Size | Use |
|---|---|---|
| `THMB` | 28×28 | list rows |
| `HEAD` | 52×52 | list headers (playlist, album) |
| `LRGE` | 116×116 | Now Playing and Home |
| `BLUR` | 80×60 | background: blurred, center-cropped to 4:3, upscaled on device |

Readers look up classes by ID, never by position, so sizes can change without a version bump.

### Pixels

RGB565, little-endian `u16`, row-major, no padding. Each image is center-cropped to the class's aspect ratio, resampled, and converted to RGB565 with Floyd–Steinberg dithering. An image that fails to decode is filled with `0x2104` (dark grey).

## Journal (draft)

The journal is `/.ipodos/journal.bin` on the device, an append-only file with one record per event. The companion reads it, merges it into host state, and truncates it after a successful sync.

| Off | Type | Field |
|---|---|---|
| 0 | u8 | type (1 = played, 2 = skipped, 3 = rating, 4 = position) |
| 1 | u8[3] | reserved |
| 4 | u32 | track uid |
| 8 | u32 | unix time |
| 12 | u32 | value (rating 0–5, or position in ms) |

Records are 16 bytes. A trailing partial record left by a power cut is discarded.
