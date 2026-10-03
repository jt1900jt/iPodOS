//! Builds anti-aliased bitmap font atlases (.ipfn) from TrueType fonts.
//! See docs/font-format.md.

use anyhow::{bail, Context, Result};
use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont};

pub const FONT_MAGIC: [u8; 4] = *b"IPFN";
pub const FONT_HEADER: usize = 64;
pub const RANGE_SIZE: usize = 8;
pub const GLYPH_SIZE: usize = 12;

/// Codepoint ranges covered by every face: Latin-1, Latin Extended-A, and the punctuation,
/// currency and arrow characters that show up in music metadata.
pub const RANGES: &[(u32, u32)] = &[
    (0x0020, 0x007E), // ASCII printable
    (0x00A0, 0x00FF), // Latin-1 supplement
    (0x0100, 0x017F), // Latin Extended-A
    (0x2010, 0x201F), // dashes and quotes
    (0x2020, 0x2022), // dagger, double dagger, bullet
    (0x2026, 0x2026), // ellipsis
    (0x20AC, 0x20AC), // euro
    (0x2122, 0x2122), // trademark
    (0x2190, 0x2193), // arrows
];

pub struct FaceSpec {
    pub name: &'static str,
    pub ttf: &'static str,
    pub px: u16,
    pub tracking: i16,
}

pub const FACES: &[FaceSpec] = &[
    FaceSpec { name: "title-18", ttf: "Inter-SemiBold.ttf", px: 18, tracking: 0 },
    FaceSpec { name: "menu-14", ttf: "Inter-SemiBold.ttf", px: 14, tracking: 1 },
    FaceSpec { name: "row-13", ttf: "Inter-SemiBold.ttf", px: 13, tracking: 0 },
    FaceSpec { name: "body-12", ttf: "Inter-Medium.ttf", px: 12, tracking: 0 },
    FaceSpec { name: "sub-11", ttf: "Inter-Regular.ttf", px: 11, tracking: 0 },
    FaceSpec { name: "caps-10", ttf: "Inter-SemiBold.ttf", px: 10, tracking: 1 },
];

struct Glyph {
    bitmap: Vec<u8>,
    w: u8,
    h: u8,
    left: i8,
    top: i8,
    advance: u8,
}

fn clamp_i8(v: f32) -> i8 {
    v.round().clamp(i8::MIN as f32, i8::MAX as f32) as i8
}

fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, u8::MAX as f32) as u8
}

fn render(font: &FontRef, scale: PxScale, id: GlyphId) -> Glyph {
    let scaled = font.as_scaled(scale);
    let advance = clamp_u8(scaled.h_advance(id));
    let glyph = id.with_scale(scale);
    match font.outline_glyph(glyph) {
        Some(outlined) => {
            let b = outlined.px_bounds();
            let (w, h) = (b.width().ceil() as usize, b.height().ceil() as usize);
            if w == 0 || h == 0 || w > 255 || h > 255 {
                return Glyph { bitmap: Vec::new(), w: 0, h: 0, left: 0, top: 0, advance };
            }
            let mut bitmap = vec![0u8; w * h];
            outlined.draw(|x, y, c| {
                let (x, y) = (x as usize, y as usize);
                if x < w && y < h {
                    bitmap[y * w + x] = clamp_u8(c * 255.0);
                }
            });
            Glyph {
                bitmap,
                w: w as u8,
                h: h as u8,
                left: clamp_i8(b.min.x),
                top: clamp_i8(-b.min.y), // px_bounds y grows downward from the baseline
                advance,
            }
        }
        None => Glyph { bitmap: Vec::new(), w: 0, h: 0, left: 0, top: 0, advance },
    }
}

/// Build one atlas. `ttf` is the raw font file.
pub fn build_face(ttf: &[u8], px: u16, tracking: i16) -> Result<Vec<u8>> {
    let font = FontRef::try_from_slice(ttf).context("parsing font")?;
    let scale = PxScale::from(px as f32);
    let scaled = font.as_scaled(scale);
    let ascent = scaled.ascent().ceil().max(0.0) as u16;
    let descent = (-scaled.descent()).ceil().max(0.0) as u16;
    let line_height = (scaled.height() + scaled.line_gap()).ceil().max(1.0) as u16;

    // Glyph 0 is the fallback: a hollow box, the height of a capital.
    let box_h = (px as usize * 2 / 3).max(3);
    let box_w = (box_h * 2 / 3).max(2);
    let mut fallback = vec![0u8; box_w * box_h];
    for x in 0..box_w {
        fallback[x] = 255;
        fallback[(box_h - 1) * box_w + x] = 255;
    }
    for y in 0..box_h {
        fallback[y * box_w] = 255;
        fallback[y * box_w + box_w - 1] = 255;
    }
    let mut glyphs = vec![Glyph {
        bitmap: fallback,
        w: box_w as u8,
        h: box_h as u8,
        left: 1,
        top: box_h as i8,
        advance: (box_w + 2) as u8,
    }];

    let mut ranges: Vec<(u32, u16, u16)> = Vec::new();
    for &(first, last) in RANGES {
        if last < first {
            bail!("bad range {first:#x}..{last:#x}");
        }
        let start = glyphs.len() as u16;
        for cp in first..=last {
            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
            let id = font.glyph_id(ch);
            // A codepoint the font lacks maps to .notdef; store the fallback metrics instead
            // so the device draws a box rather than whatever .notdef happens to be.
            if id.0 == 0 && ch != '\u{0}' {
                let adv = clamp_u8(scaled.h_advance(font.glyph_id(' ')));
                glyphs.push(Glyph { bitmap: Vec::new(), w: 0, h: 0, left: 0, top: 0, advance: adv });
            } else {
                glyphs.push(render(&font, scale, id));
            }
        }
        let count = (last - first + 1) as u16;
        ranges.push((first, count, start));
    }

    if glyphs.len() > u16::MAX as usize {
        bail!("too many glyphs");
    }

    let mut bitmaps: Vec<u8> = Vec::new();
    let mut glyph_table = Vec::with_capacity(glyphs.len() * GLYPH_SIZE);
    for g in &glyphs {
        let off = if g.bitmap.is_empty() { 0 } else { bitmaps.len() as u32 };
        bitmaps.extend_from_slice(&g.bitmap);
        glyph_table.extend_from_slice(&off.to_le_bytes());
        glyph_table.extend_from_slice(&[g.w, g.h, g.left as u8, g.top as u8, g.advance, 0]);
        glyph_table.extend_from_slice(&0u16.to_le_bytes());
    }

    let mut range_table = Vec::with_capacity(ranges.len() * RANGE_SIZE);
    for (first, count, start) in &ranges {
        range_table.extend_from_slice(&first.to_le_bytes());
        range_table.extend_from_slice(&count.to_le_bytes());
        range_table.extend_from_slice(&start.to_le_bytes());
    }

    let range_offset = FONT_HEADER;
    let glyph_offset = range_offset + range_table.len();
    let bitmap_offset = glyph_offset + glyph_table.len();
    let file_size = bitmap_offset + bitmaps.len();

    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(&FONT_MAGIC);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(FONT_HEADER as u32).to_le_bytes());
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // crc, filled below
    out.extend_from_slice(&px.to_le_bytes());
    out.extend_from_slice(&ascent.to_le_bytes());
    out.extend_from_slice(&descent.to_le_bytes());
    out.extend_from_slice(&line_height.to_le_bytes());
    out.extend_from_slice(&(glyphs.len() as u16).to_le_bytes());
    out.extend_from_slice(&(ranges.len() as u16).to_le_bytes());
    out.extend_from_slice(&tracking.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(range_offset as u32).to_le_bytes());
    out.extend_from_slice(&(glyph_offset as u32).to_le_bytes());
    out.extend_from_slice(&(bitmap_offset as u32).to_le_bytes());
    out.extend_from_slice(&(bitmaps.len() as u32).to_le_bytes());
    out.resize(FONT_HEADER, 0);
    out.extend_from_slice(&range_table);
    out.extend_from_slice(&glyph_table);
    out.extend_from_slice(&bitmaps);
    debug_assert_eq!(out.len(), file_size);

    let crc = crc32fast::hash(&out[FONT_HEADER..]);
    out[16..20].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    const TTF: &[u8] = include_bytes!("../assets/Inter-SemiBold.ttf");

    #[test]
    fn header_and_tables_are_consistent() {
        let f = build_face(TTF, 13, 0).unwrap();
        assert_eq!(&f[0..4], b"IPFN");
        assert_eq!(u32_at(&f, 12) as usize, f.len());
        assert_eq!(crc32fast::hash(&f[FONT_HEADER..]), u32_at(&f, 16));
        assert_eq!(u16_at(&f, 20), 13);
        assert!(u16_at(&f, 22) > 0 && u16_at(&f, 26) > 0);

        let n_glyphs = u16_at(&f, 28) as usize;
        let n_ranges = u16_at(&f, 30) as usize;
        assert_eq!(n_ranges, RANGES.len());
        let (ro, go, bo, bs) =
            (u32_at(&f, 36) as usize, u32_at(&f, 40) as usize, u32_at(&f, 44) as usize, u32_at(&f, 48) as usize);
        assert_eq!(go - ro, n_ranges * RANGE_SIZE);
        assert_eq!(bo - go, n_glyphs * GLYPH_SIZE);
        assert_eq!(bo + bs, f.len());

        // every glyph's bitmap lies inside the bitmap section
        for i in 0..n_glyphs {
            let g = go + i * GLYPH_SIZE;
            let (off, w, h) = (u32_at(&f, g) as usize, f[g + 4] as usize, f[g + 5] as usize);
            assert!(off + w * h <= bs, "glyph {i} bitmap out of range");
        }
        // ranges are sorted, non-overlapping, and index real glyphs
        let mut prev_end = 0;
        for i in 0..n_ranges {
            let r = ro + i * RANGE_SIZE;
            let (first, count, start) = (u32_at(&f, r), u16_at(&f, r + 4) as usize, u16_at(&f, r + 6) as usize);
            assert!(first >= prev_end, "ranges out of order");
            prev_end = first + count as u32;
            assert!(start + count <= n_glyphs);
        }
    }

    #[test]
    fn glyphs_have_sane_metrics() {
        let f = build_face(TTF, 13, 0).unwrap();
        let go = u32_at(&f, 40) as usize;
        let ro = u32_at(&f, 36) as usize;
        // 'A' is in the first range (ASCII), so its index is derivable
        let start = u16_at(&f, ro + 6) as usize;
        let idx = start + (b'A' as usize - 0x20);
        let g = go + idx * GLYPH_SIZE;
        let (w, h, adv) = (f[g + 4], f[g + 5], f[g + 8]);
        assert!(w > 2 && h > 5, "A should have ink: {w}x{h}");
        assert!(adv >= w, "advance {adv} < width {w}");

        // space has no ink but does advance
        let sp = go + start * GLYPH_SIZE;
        assert_eq!(f[sp + 4], 0);
        assert!(f[sp + 8] > 0);

        // fallback box is glyph 0
        assert!(f[go + 4] > 0 && f[go + 5] > 0);
    }

    #[test]
    fn coverage_is_antialiased() {
        let f = build_face(TTF, 18, 0).unwrap();
        let (go, bo) = (u32_at(&f, 40) as usize, u32_at(&f, 44) as usize);
        let ro = u32_at(&f, 36) as usize;
        let start = u16_at(&f, ro + 6) as usize;
        let idx = start + (b'S' as usize - 0x20);
        let g = go + idx * GLYPH_SIZE;
        let (off, w, h) = (u32_at(&f, g) as usize, f[g + 4] as usize, f[g + 5] as usize);
        let px = &f[bo + off..bo + off + w * h];
        let partial = px.iter().filter(|&&v| v > 0 && v < 255).count();
        assert!(partial > w, "expected anti-aliased edges, got {partial} partial pixels");
    }
}

/// Reader for a built atlas. Mirrors the device-side reader in apps/shell/ipfn.c, and is
/// used by the CLI's preview rendering and by the tests.
pub struct Atlas<'a> {
    ranges: &'a [u8],
    glyphs: &'a [u8],
    bitmaps: &'a [u8],
    n_ranges: usize,
    n_glyphs: usize,
    px: u16,
    ascent: u16,
    line_height: u16,
    tracking: i16,
}

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl<'a> Atlas<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < FONT_HEADER || data[0..4] != FONT_MAGIC {
            bail!("not an ipfn font");
        }
        if rd16(data, 4) != 1 {
            bail!("unsupported font version");
        }
        let file_size = rd32(data, 12) as usize;
        if file_size > data.len() {
            bail!("font truncated");
        }
        if crc32fast::hash(&data[FONT_HEADER..file_size]) != rd32(data, 16) {
            bail!("font crc mismatch");
        }
        let n_glyphs = rd16(data, 28) as usize;
        let n_ranges = rd16(data, 30) as usize;
        let (ro, go, bo, bs) = (
            rd32(data, 36) as usize,
            rd32(data, 40) as usize,
            rd32(data, 44) as usize,
            rd32(data, 48) as usize,
        );
        if go != ro + n_ranges * RANGE_SIZE || bo != go + n_glyphs * GLYPH_SIZE || bo + bs != file_size {
            bail!("font tables inconsistent");
        }
        Ok(Atlas {
            ranges: &data[ro..go],
            glyphs: &data[go..bo],
            bitmaps: &data[bo..bo + bs],
            n_ranges,
            n_glyphs,
            px: rd16(data, 20),
            ascent: rd16(data, 22),
            line_height: rd16(data, 26),
            tracking: rd16(data, 32) as i16,
        })
    }

    pub fn px(&self) -> u16 {
        self.px
    }
    pub fn ascent(&self) -> u16 {
        self.ascent
    }
    pub fn line_height(&self) -> u16 {
        self.line_height
    }

    fn index(&self, cp: u32) -> usize {
        for i in 0..self.n_ranges {
            let r = i * RANGE_SIZE;
            let first = rd32(self.ranges, r);
            let count = rd16(self.ranges, r + 4) as u32;
            if cp >= first && cp < first + count {
                return rd16(self.ranges, r + 6) as usize + (cp - first) as usize;
            }
        }
        0
    }

    /// (bitmap, w, h, left, top, advance)
    fn glyph(&self, idx: usize) -> (&'a [u8], usize, usize, i32, i32, i32) {
        let g = idx.min(self.n_glyphs - 1) * GLYPH_SIZE;
        let off = rd32(self.glyphs, g) as usize;
        let w = self.glyphs[g + 4] as usize;
        let h = self.glyphs[g + 5] as usize;
        let left = self.glyphs[g + 6] as i8 as i32;
        let top = self.glyphs[g + 7] as i8 as i32;
        let adv = self.glyphs[g + 8] as i32;
        let bm = if w * h == 0 { &self.bitmaps[0..0] } else { &self.bitmaps[off..off + w * h] };
        (bm, w, h, left, top, adv)
    }

    pub fn measure(&self, text: &str) -> i32 {
        let mut x = 0;
        for ch in text.chars() {
            let (_, _, _, _, _, adv) = self.glyph(self.index(ch as u32));
            x += adv + self.tracking as i32;
        }
        (x - self.tracking as i32).max(0)
    }

    /// Walks the string and hands each covered pixel to `put(x, y, coverage)`.
    /// `y` is the baseline.
    pub fn draw(&self, text: &str, x0: i32, baseline: i32, mut put: impl FnMut(i32, i32, u8)) {
        let mut pen = x0;
        for ch in text.chars() {
            let (bm, w, h, left, top, adv) = self.glyph(self.index(ch as u32));
            for row in 0..h {
                for col in 0..w {
                    let a = bm[row * w + col];
                    if a != 0 {
                        put(pen + left + col as i32, baseline - top + row as i32, a);
                    }
                }
            }
            pen += adv + self.tracking as i32;
        }
    }
}

#[cfg(test)]
mod atlas_tests {
    use super::*;

    const TTF: &[u8] = include_bytes!("../assets/Inter-SemiBold.ttf");

    #[test]
    fn round_trip() {
        let f = build_face(TTF, 13, 1).unwrap();
        let a = Atlas::parse(&f).unwrap();
        assert_eq!(a.px(), 13);
        assert!(a.measure("Halcyon Drift") > 40);
        assert_eq!(a.measure(""), 0);

        let mut ink = 0;
        let (mut minx, mut maxx) = (i32::MAX, i32::MIN);
        a.draw("Ag", 0, a.ascent() as i32, |x, _y, v| {
            ink += 1;
            minx = minx.min(x);
            maxx = maxx.max(x);
            assert!(v > 0);
        });
        assert!(ink > 20, "expected ink, got {ink}");
        assert!(minx >= -2 && maxx <= a.measure("Ag") + 2, "glyphs outside advance box");
    }

    #[test]
    fn rejects_corruption() {
        let good = build_face(TTF, 11, 0).unwrap();
        assert!(Atlas::parse(&good[..40]).is_err());
        let mut bad = good.clone();
        bad[FONT_HEADER + 4] ^= 0xFF;
        assert!(Atlas::parse(&bad).is_err(), "crc should catch body corruption");
        let mut bad2 = good.clone();
        bad2[0] = b'X';
        assert!(Atlas::parse(&bad2).is_err());
    }

    #[test]
    fn unknown_codepoint_falls_back_to_box() {
        let f = build_face(TTF, 13, 0).unwrap();
        let a = Atlas::parse(&f).unwrap();
        assert_eq!(a.index(0x4E00), 0, "CJK is outside the ranges");
        let mut ink = 0;
        a.draw("\u{4E00}", 0, 10, |_, _, _| ink += 1);
        assert!(ink > 0, "fallback box should draw something");
    }
}
