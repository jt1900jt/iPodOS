//! Art pack writer: decodes covers once, renders every class as dithered RGB565,
//! and extracts per-album dominant colors.

use std::io::{Seek, SeekFrom, Write};

use anyhow::Result;
use image::imageops::{self, FilterType};
use image::RgbImage;

use crate::format::*;

/// Largest dimension kept after the fast pre-shrink: 2x the largest class.
const PRESHRINK: u32 = 232;
const BLUR_SIGMA: f32 = 3.0;

fn crop_to_aspect(img: &RgbImage, w: u32, h: u32) -> RgbImage {
    let (iw, ih) = img.dimensions();
    // Compare iw/ih with w/h without floats.
    let (cw, ch) = if (iw as u64) * (h as u64) > (ih as u64) * (w as u64) {
        (((ih as u64 * w as u64) / h as u64).max(1) as u32, ih)
    } else {
        (iw, ((iw as u64 * h as u64) / w as u64).max(1) as u32)
    };
    imageops::crop_imm(img, (iw - cw) / 2, (ih - ch) / 2, cw, ch).to_image()
}

/// Floyd-Steinberg, serpentine, to little-endian RGB565.
pub fn dither_rgb565(img: &RgbImage) -> Vec<u8> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut px: Vec<[f32; 3]> = img.pixels().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    let mut out = vec![0u8; w * h * 2];
    let levels = [31.0f32, 63.0, 31.0];
    for y in 0..h {
        let ltr = y % 2 == 0;
        for xi in 0..w {
            let x = if ltr { xi } else { w - 1 - xi };
            let i = y * w + x;
            let mut q = [0u16; 3];
            let mut err = [0f32; 3];
            for c in 0..3 {
                let v = px[i][c].clamp(0.0, 255.0);
                let lv = (v * levels[c] / 255.0).round();
                q[c] = lv as u16;
                err[c] = v - lv * 255.0 / levels[c];
            }
            let v = (q[0] << 11) | (q[1] << 5) | q[2];
            out[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
            let fwd: isize = if ltr { 1 } else { -1 };
            let mut spread = |dx: isize, dy: usize, wgt: f32| {
                let nx = x as isize + dx;
                let ny = y + dy;
                if nx >= 0 && (nx as usize) < w && ny < h {
                    let j = ny * w + nx as usize;
                    for c in 0..3 {
                        px[j][c] += err[c] * wgt;
                    }
                }
            };
            spread(fwd, 0, 7.0 / 16.0);
            spread(-fwd, 1, 3.0 / 16.0);
            spread(0, 1, 5.0 / 16.0);
            spread(fwd, 1, 1.0 / 16.0);
        }
    }
    out
}

pub fn rgb565(c: [f32; 3]) -> u16 {
    let r = (c[0].clamp(0.0, 255.0) * 31.0 / 255.0).round() as u16;
    let g = (c[1].clamp(0.0, 255.0) * 63.0 / 255.0).round() as u16;
    let b = (c[2].clamp(0.0, 255.0) * 31.0 / 255.0).round() as u16;
    (r << 11) | (g << 5) | b
}

/// Three dominant colors by k-means on a 24x24 sample, most populous first.
pub fn dominant_colors(img: &RgbImage) -> [u16; 3] {
    let small = imageops::resize(img, 24, 24, FilterType::Triangle);
    let pts: Vec<[f32; 3]> = small.pixels().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    let d2 = |a: &[f32; 3], b: &[f32; 3]| (0..3).map(|c| (a[c] - b[c]).powi(2)).sum::<f32>();

    // Deterministic farthest-point init from the mean.
    let mean = {
        let mut m = [0f32; 3];
        for p in &pts {
            for c in 0..3 {
                m[c] += p[c] / pts.len() as f32;
            }
        }
        m
    };
    let mut centers = vec![mean];
    while centers.len() < 3 {
        let far = pts
            .iter()
            .max_by(|a, b| {
                let da = centers.iter().map(|c| d2(a, c)).fold(f32::MAX, f32::min);
                let db = centers.iter().map(|c| d2(b, c)).fold(f32::MAX, f32::min);
                da.total_cmp(&db)
            })
            .copied()
            .unwrap_or(mean);
        centers.push(far);
    }

    let mut counts = [0usize; 3];
    for _ in 0..12 {
        let mut sums = [[0f32; 3]; 3];
        counts = [0; 3];
        for p in &pts {
            let k = (0..3).min_by(|&a, &b| d2(p, &centers[a]).total_cmp(&d2(p, &centers[b]))).unwrap();
            counts[k] += 1;
            for c in 0..3 {
                sums[k][c] += p[c];
            }
        }
        for k in 0..3 {
            if counts[k] > 0 {
                for c in 0..3 {
                    centers[k][c] = sums[k][c] / counts[k] as f32;
                }
            }
        }
    }
    let mut ks = [0usize, 1, 2];
    ks.sort_by(|&a, &b| counts[b].cmp(&counts[a]).then(a.cmp(&b)));
    [rgb565(centers[ks[0]]), rgb565(centers[ks[1]]), rgb565(centers[ks[2]])]
}

fn render_class(src: &RgbImage, class: &ArtClass) -> Vec<u8> {
    let cropped = crop_to_aspect(src, class.width, class.height);
    let img = if class.blur {
        let small = imageops::resize(&cropped, class.width, class.height, FilterType::Triangle);
        imageops::blur(&small, BLUR_SIGMA)
    } else {
        imageops::resize(&cropped, class.width, class.height, FilterType::Lanczos3)
    };
    dither_rgb565(&img)
}

fn fallback_slot(class: &ArtClass) -> Vec<u8> {
    ART_FALLBACK_PIXEL.to_le_bytes().repeat((class.width * class.height) as usize)
}

pub struct PackLayout {
    pub data_offset: u64,
    pub class_offsets: Vec<u64>,
    pub file_size: u64,
}

pub fn layout(art_count: usize) -> PackLayout {
    let table_end = HEADER_SIZE as u64 + ART_CLASSES.len() as u64 * ART_CLASS_ENTRY_SIZE as u64;
    let data_offset = align_up(table_end as usize, ART_DATA_ALIGN as usize) as u64;
    let mut cur = data_offset;
    let mut class_offsets = Vec::new();
    for c in &ART_CLASSES {
        cur = align_up(cur as usize, ART_DATA_ALIGN as usize) as u64;
        class_offsets.push(cur);
        cur += c.slot_size() * art_count as u64;
    }
    PackLayout { data_offset, class_offsets, file_size: cur }
}

fn header_bytes(art_count: usize, generation: u64, lay: &PackLayout) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(&ART_MAGIC);
    h.extend_from_slice(&VERSION_MAJOR.to_le_bytes());
    h.extend_from_slice(&VERSION_MINOR.to_le_bytes());
    h.extend_from_slice(&HEADER_SIZE.to_le_bytes());
    h.extend_from_slice(&(ART_CLASSES.len() as u32).to_le_bytes());
    h.extend_from_slice(&(art_count as u32).to_le_bytes());
    h.extend_from_slice(&(lay.data_offset as u32).to_le_bytes());
    h.extend_from_slice(&generation.to_le_bytes());
    h.extend_from_slice(&lay.file_size.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes()); // crc placeholder
    h.resize(HEADER_SIZE as usize, 0);
    for (c, &off) in ART_CLASSES.iter().zip(&lay.class_offsets) {
        h.extend_from_slice(&c.id);
        h.extend_from_slice(&(c.width as u16).to_le_bytes());
        h.extend_from_slice(&(c.height as u16).to_le_bytes());
        h.extend_from_slice(&off.to_le_bytes());
    }
    let crc = crc32fast::hash(&h);
    h[40..44].copy_from_slice(&crc.to_le_bytes());
    h
}

/// Write the art pack. `load(i)` returns the encoded image for art_id `i`.
/// Images that fail to load or decode get a flat fallback and zero colors; `warn` is told why.
pub fn write_pack<W: Write + Seek>(
    out: &mut W,
    art_count: usize,
    generation: u64,
    mut load: impl FnMut(usize) -> Result<Vec<u8>>,
    mut warn: impl FnMut(usize, String),
) -> Result<Vec<[u16; 3]>> {
    let lay = layout(art_count);
    let head = header_bytes(art_count, generation, &lay);
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&head)?;
    out.write_all(&vec![0u8; (lay.data_offset as usize).saturating_sub(head.len())])?;

    let mut colors = Vec::with_capacity(art_count);
    for i in 0..art_count {
        let decoded = load(i).and_then(|bytes| Ok(image::load_from_memory(&bytes)?.to_rgb8()));
        let slots: Vec<Vec<u8>> = match decoded {
            Ok(img) => {
                let img = if img.width().max(img.height()) > PRESHRINK {
                    let (w, h) = img.dimensions();
                    let s = PRESHRINK as f32 / w.max(h) as f32;
                    imageops::thumbnail(&img, ((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1))
                } else {
                    img
                };
                colors.push(dominant_colors(&img));
                ART_CLASSES.iter().map(|c| render_class(&img, c)).collect()
            }
            Err(e) => {
                warn(i, format!("{e:#}"));
                colors.push([0; 3]);
                ART_CLASSES.iter().map(fallback_slot).collect()
            }
        };
        for ((c, &off), data) in ART_CLASSES.iter().zip(&lay.class_offsets).zip(&slots) {
            out.seek(SeekFrom::Start(off + i as u64 * c.slot_size()))?;
            out.write_all(data)?;
        }
    }
    // Make the file reach file_size even if the last class region was written earlier.
    if art_count > 0 {
        let cur_len = out.seek(SeekFrom::End(0))?;
        if cur_len < lay.file_size {
            out.write_all(&vec![0u8; (lay.file_size - cur_len) as usize])?;
        }
    }
    out.flush()?;
    Ok(colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dither_extremes() {
        let img = RgbImage::from_pixel(4, 4, image::Rgb([255, 255, 255]));
        assert!(dither_rgb565(&img).chunks(2).all(|p| u16::from_le_bytes([p[0], p[1]]) == 0xFFFF));
        let img = RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 0]));
        assert!(dither_rgb565(&img).iter().all(|&b| b == 0));
    }

    #[test]
    fn colors_of_two_tone() {
        let mut img = RgbImage::from_pixel(32, 32, image::Rgb([255, 0, 0]));
        for y in 0..32 {
            for x in 0..8 {
                img.put_pixel(x, y, image::Rgb([0, 0, 255]));
            }
        }
        assert_eq!(dominant_colors(&img)[0], 0xF800);
    }

    #[test]
    fn crop_aspect() {
        let img = RgbImage::new(200, 100);
        assert_eq!(crop_to_aspect(&img, 1, 1).dimensions(), (100, 100));
        assert_eq!(crop_to_aspect(&img, 4, 3).dimensions(), (133, 100));
    }
}
