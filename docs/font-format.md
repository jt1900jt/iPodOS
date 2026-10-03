# Font atlas format (v1.0)

Anti-aliased bitmap fonts, rendered on the host at the exact pixel sizes the UI uses and
stored as 8-bit coverage. The device blends each glyph over whatever is behind it, so text
stays sharp over art, gradients and the selection bar alike.

Rockbox's own `.fnt` fonts are 1-bit, which is what makes stock Rockbox text look ragged at
these sizes. These replace them in the shell.

One file per face and size: `/.ipodos/fonts/<name>.ipfn`. Built by `ipdb fonts`.

## Header (64 bytes)

| Off | Type | Field |
|---|---|---|
| 0 | char[4] | magic `IPFN` |
| 4 | u16 | version_major (1) |
| 6 | u16 | version_minor (0) |
| 8 | u32 | header_size (64) |
| 12 | u32 | file_size |
| 16 | u32 | crc32 (IEEE) of bytes `[header_size, file_size)` |
| 20 | u16 | px (nominal pixel size) |
| 22 | u16 | ascent (px above the baseline) |
| 24 | u16 | descent (px below the baseline) |
| 26 | u16 | line_height |
| 28 | u16 | glyph_count |
| 30 | u16 | range_count |
| 32 | i16 | tracking (extra px between glyphs, signed) |
| 34 | u16 | flags (0) |
| 36 | u32 | range_offset |
| 40 | u32 | glyph_offset |
| 44 | u32 | bitmap_offset |
| 48 | u32 | bitmap_size |
| 52 | u8[12] | reserved, zero |

## Ranges

`range_count` entries of 8 bytes at `range_offset`, sorted by `first`, non-overlapping:

| Off | Type | Field |
|---|---|---|
| 0 | u32 | first codepoint |
| 4 | u16 | count |
| 6 | u16 | glyph_index of `first` |

A codepoint's glyph index is `glyph_index + (cp - first)`. Codepoints outside every range
fall back to glyph 0, which is always present and is the replacement box.

Splitting the character set into ranges keeps the lookup table small: Latin, punctuation and
symbols are contiguous blocks with large gaps between them.

## Glyphs

`glyph_count` entries of 12 bytes at `glyph_offset`:

| Off | Type | Field |
|---|---|---|
| 0 | u32 | bitmap offset, relative to `bitmap_offset` |
| 4 | u8 | width |
| 5 | u8 | height |
| 6 | i8 | left (x offset from the pen) |
| 7 | i8 | top (y offset from the baseline, up positive) |
| 8 | u8 | advance |
| 9 | u8 | reserved |
| 10 | u16 | reserved |

## Bitmaps

8-bit coverage, one byte per pixel, row-major, `width * height` bytes per glyph, no padding
and no row alignment. A glyph with no ink (space) has `width = height = 0` and its bitmap
offset is ignored.

Coverage is linear, so the device blends with `out = bg + (fg - bg) * a / 255` per channel.

## Rendering on the device

Text is drawn by walking the string's codepoints, looking up each glyph, blending its
coverage over the destination, and advancing the pen by `advance + tracking`. There is no
kerning: Inter's default spacing is even enough at these sizes, and a kerning table would
cost more than it gains.

## Faces

Built from Inter 4.1 (SIL Open Font License). The sizes match the UI layout:

| File | Face | px | Use |
|---|---|---|---|
| `title-18.ipfn` | SemiBold | 18 | Now Playing title, playlist header |
| `row-13.ipfn` | SemiBold | 13 | list row titles |
| `body-12.ipfn` | Medium | 12 | Now Playing artist |
| `sub-11.ipfn` | Regular | 11 | list row subtitles |
| `caps-10.ipfn` | SemiBold | 10 | status bar, uppercase labels, durations |
| `menu-14.ipfn` | SemiBold | 14 | home menu items |

`caps-10` and `menu-14` carry positive tracking to match the letterspaced uppercase in the
design.
