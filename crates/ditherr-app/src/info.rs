//! The Studio info panel (spec §8): a tone histogram, which colours the result
//! uses and how much, and an estimated file size.

use std::collections::HashMap;

use ditherr_core::color::{linear_to_srgb, luminance};
use ditherr_core::{ImageF32, Rendered};

/// Histogram resolution: bars from black to white.
pub const HISTOGRAM_BINS: usize = 64;
/// Colours listed individually; the rest are summed into "other".
pub const TOP_COLORS: usize = 12;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stats {
    /// Tones going into the dither: perceptual luma of the pre-processed
    /// image, `HISTOGRAM_BINS` bars from black to white.
    pub histogram: Vec<u32>,
    /// Distinct colours in the result.
    pub colors: usize,
    /// The most used colours, most used first, with their pixel counts.
    pub top_colors: Vec<([u8; 3], u32)>,
    /// The result's whole palette in code order. Empty when the palette is
    /// too large to index (above 256 colours).
    pub palette: Vec<[u8; 3]>,
    /// Pixels in the result.
    pub pixels: u64,
    /// Size of the result written as PNG at the size that was rendered.
    pub png_bytes: usize,
}

/// Perceptual luma histogram of a working image.
pub fn luma_histogram(img: &ImageF32) -> Vec<u32> {
    let mut bins = vec![0u32; HISTOGRAM_BINS];
    for p in &img.data {
        // Transparent pixels are composited onto the background, so they say
        // nothing about tone; skip them.
        if p[3] <= 0.0 {
            continue;
        }
        let y = linear_to_srgb(luminance([p[0], p[1], p[2]]).clamp(0.0, 1.0));
        let i = ((y * HISTOGRAM_BINS as f32) as usize).min(HISTOGRAM_BINS - 1);
        bins[i] += 1;
    }
    bins
}

/// Per-colour pixel counts of a result, most used first (ties by colour value).
pub fn colour_usage(img: &Rendered) -> Vec<([u8; 3], u32)> {
    let mut usage: Vec<([u8; 3], u32)> = match img {
        Rendered::Indexed(i) => {
            let mut counts = vec![0u32; i.palette.len().max(256)];
            for &ix in &i.indices {
                counts[usize::from(ix)] += 1;
            }
            i.palette
                .iter()
                .zip(counts)
                .filter(|(_, n)| *n > 0)
                .map(|(c, n)| (*c, n))
                .collect()
        }
        Rendered::Truecolor(t) => {
            let mut map: HashMap<[u8; 3], u32> = HashMap::new();
            for px in t.data.chunks_exact(4) {
                *map.entry([px[0], px[1], px[2]]).or_insert(0) += 1;
            }
            map.into_iter().collect()
        }
    };
    // Palettes may list a colour twice; count it once.
    usage.sort_by_key(|&(c, _)| c);
    // `dedup_by` hands over the later element first; fold it into the earlier one.
    usage.dedup_by(|later, earlier| {
        if earlier.0 == later.0 {
            earlier.1 += later.1;
            true
        } else {
            false
        }
    });
    usage.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    usage
}

/// Statistics for a finished render. `png_bytes` is the encoded size of `img`
/// itself, so pass the image at the size you want to report.
pub fn analyze(working: &ImageF32, img: &Rendered, png_bytes: usize) -> Stats {
    let usage = colour_usage(img);
    let palette = match img {
        Rendered::Indexed(i) => i.palette.clone(),
        Rendered::Truecolor(_) => Vec::new(),
    };
    Stats {
        histogram: luma_histogram(working),
        colors: usage.len(),
        palette,
        pixels: u64::from(img.width()) * u64::from(img.height()),
        top_colors: usage.into_iter().take(TOP_COLORS).collect(),
        png_bytes,
    }
}

/// Scales the size measured on a preview up to a full-size render, assuming
/// the same bytes per pixel. `scale` is full pixels over preview pixels.
pub fn estimate_full_size(png_bytes: usize, scale: f64) -> usize {
    (png_bytes as f64 * scale.max(0.0)).round() as usize
}

/// "812 B", "14.2 KB", "3.1 MB".
pub fn format_bytes(n: usize) -> String {
    const KB: f64 = 1024.0;
    let n = n as f64;
    if n < KB {
        format!("{n:.0} B")
    } else if n < KB * KB {
        format!("{:.1} KB", n / KB)
    } else {
        format!("{:.1} MB", n / (KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditherr_core::{IndexedImage, Rgba8Image};

    fn indexed(indices: Vec<u8>, palette: Vec<[u8; 3]>) -> Rendered {
        let w = indices.len() as u32;
        Rendered::Indexed(IndexedImage {
            width: w,
            height: 1,
            indices,
            palette,
        })
    }

    #[test]
    fn usage_counts_and_orders_colours() {
        let img = indexed(
            vec![0, 1, 1, 1, 2, 2],
            vec![[0, 0, 0], [255, 0, 0], [0, 0, 255], [9, 9, 9]],
        );
        // The fourth palette entry is never used.
        assert_eq!(
            colour_usage(&img),
            vec![([255, 0, 0], 3), ([0, 0, 255], 2), ([0, 0, 0], 1)]
        );
    }

    #[test]
    fn duplicate_palette_entries_count_once() {
        let img = indexed(vec![0, 1, 1], vec![[5, 5, 5], [5, 5, 5]]);
        assert_eq!(colour_usage(&img), vec![([5, 5, 5], 3)]);
    }

    #[test]
    fn truecolour_usage() {
        let data = vec![1, 2, 3, 255, 1, 2, 3, 255, 9, 9, 9, 255];
        let img = Rendered::Truecolor(Rgba8Image::new(3, 1, data).unwrap());
        assert_eq!(colour_usage(&img), vec![([1, 2, 3], 2), ([9, 9, 9], 1)]);
    }

    #[test]
    fn histogram_puts_black_and_white_at_the_ends() {
        let mut img = ImageF32::new(4, 1);
        img.data = vec![
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            // Fully transparent: ignored.
            [1.0, 1.0, 1.0, 0.0],
        ];
        let h = luma_histogram(&img);
        assert_eq!(h.len(), HISTOGRAM_BINS);
        assert_eq!(h[0], 2);
        assert_eq!(h[HISTOGRAM_BINS - 1], 1);
        assert_eq!(h.iter().sum::<u32>(), 3);
    }

    #[test]
    fn mid_gray_lands_mid_histogram() {
        // sRGB 50% gray is about 21% linear.
        let mut img = ImageF32::new(1, 1);
        img.data = vec![[0.2140, 0.2140, 0.2140, 1.0]];
        let h = luma_histogram(&img);
        let bin = h.iter().position(|&n| n == 1).unwrap();
        assert!(
            (HISTOGRAM_BINS / 2 - 1..=HISTOGRAM_BINS / 2).contains(&bin),
            "{bin}"
        );
    }

    #[test]
    fn analyze_limits_the_colour_list_but_counts_all() {
        let n = TOP_COLORS + 5;
        let palette: Vec<[u8; 3]> = (0..n).map(|i| [i as u8 * 10, 0, 0]).collect();
        let indices: Vec<u8> = (0..n as u8).collect();
        let img = indexed(indices, palette);
        let stats = analyze(&ImageF32::new(1, 1), &img, 77);
        assert_eq!(stats.colors, n);
        assert_eq!(stats.palette.len(), n);
        assert_eq!(stats.top_colors.len(), TOP_COLORS);
        assert_eq!(stats.pixels, n as u64);
        assert_eq!(stats.png_bytes, 77);
    }

    #[test]
    fn size_formatting_and_estimate() {
        assert_eq!(format_bytes(812), "812 B");
        assert_eq!(format_bytes(14_540), "14.2 KB");
        assert_eq!(format_bytes(3_250_000), "3.1 MB");
        assert_eq!(estimate_full_size(1000, 4.0), 4000);
        assert_eq!(estimate_full_size(1000, -1.0), 0);
    }
}
