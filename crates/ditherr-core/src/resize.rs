//! Separable resampling on linear-light RGBA, premultiplied for correct edges.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::buffer::ImageF32;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resampler {
    Nearest,
    Bilinear,
    /// Catmull-Rom.
    Bicubic,
    #[default]
    Lanczos,
}

impl Resampler {
    fn support(self) -> f32 {
        match self {
            Resampler::Nearest => 0.5,
            Resampler::Bilinear => 1.0,
            Resampler::Bicubic => 2.0,
            Resampler::Lanczos => 3.0,
        }
    }

    fn weight(self, x: f32) -> f32 {
        let x = x.abs();
        match self {
            Resampler::Nearest => 1.0,
            Resampler::Bilinear => (1.0 - x).max(0.0),
            Resampler::Bicubic => {
                if x < 1.0 {
                    1.5 * x * x * x - 2.5 * x * x + 1.0
                } else if x < 2.0 {
                    -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
                } else {
                    0.0
                }
            }
            Resampler::Lanczos => {
                if x < 1e-6 {
                    1.0
                } else if x < 3.0 {
                    let px = std::f32::consts::PI * x;
                    3.0 * libm::sinf(px) * libm::sinf(px / 3.0) / (px * px)
                } else {
                    0.0
                }
            }
        }
    }
}

/// How the new size is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ResizeTarget {
    Pixels {
        width: u32,
        height: u32,
    },
    /// Scale both axes by a percentage (100 = unchanged).
    Percent(f32),
    /// Fixed width, aspect ratio kept.
    Width(u32),
    /// Fixed height, aspect ratio kept.
    Height(u32),
    /// Fit inside a box, aspect ratio kept; never enlarges.
    Fit {
        max_width: u32,
        max_height: u32,
    },
}

impl ResizeTarget {
    pub fn resolve(self, w: u32, h: u32) -> Result<(u32, u32)> {
        let scaled = |v: u32, s: f32| ((v as f32 * s).round() as u32).max(1);
        let (nw, nh) = match self {
            ResizeTarget::Pixels { width, height } => (width, height),
            ResizeTarget::Percent(p) => {
                if !(p > 0.0 && p.is_finite()) {
                    return Err(Error::InvalidParam("resize percent must be > 0".into()));
                }
                (scaled(w, p / 100.0), scaled(h, p / 100.0))
            }
            ResizeTarget::Width(nw) => (nw, scaled(h, nw as f32 / w as f32)),
            ResizeTarget::Height(nh) => (scaled(w, nh as f32 / h as f32), nh),
            ResizeTarget::Fit {
                max_width,
                max_height,
            } => {
                let s = (max_width as f32 / w as f32)
                    .min(max_height as f32 / h as f32)
                    .min(1.0);
                (scaled(w, s), scaled(h, s))
            }
        };
        if nw == 0 || nh == 0 {
            return Err(Error::InvalidParam("resize target has a zero side".into()));
        }
        Ok((nw, nh))
    }
}

struct Contribution {
    start: usize,
    weights: Vec<f32>,
}

fn contributions(src: usize, dst: usize, r: Resampler) -> Vec<Contribution> {
    let scale = src as f32 / dst as f32;
    let (filter_scale, support) = if scale > 1.0 {
        (scale, r.support() * scale)
    } else {
        (1.0, r.support())
    };
    (0..dst)
        .map(|o| {
            if r == Resampler::Nearest {
                let i = (((o as f32 + 0.5) * scale) as usize).min(src - 1);
                return Contribution {
                    start: i,
                    weights: vec![1.0],
                };
            }
            let center = (o as f32 + 0.5) * scale;
            let lo = ((center - support).floor() as isize).max(0) as usize;
            let hi = (((center + support).ceil() as isize).min(src as isize)) as usize;
            let mut weights: Vec<f32> = (lo..hi)
                .map(|i| r.weight((i as f32 + 0.5 - center) / filter_scale))
                .collect();
            let sum: f32 = weights.iter().sum();
            if sum.abs() > 1e-8 {
                for w in &mut weights {
                    *w /= sum;
                }
            }
            Contribution { start: lo, weights }
        })
        .collect()
}

/// Resamples to `width x height`.
pub fn resize(img: &ImageF32, width: u32, height: u32, r: Resampler) -> Result<ImageF32> {
    if width == 0 || height == 0 || img.is_empty() {
        return Err(Error::EmptyImage);
    }
    if width == img.width && height == img.height {
        return Ok(img.clone());
    }
    let (sw, sh, dw, dh) = (
        img.width as usize,
        img.height as usize,
        width as usize,
        height as usize,
    );
    let mut src = img.clone();
    src.premultiply();

    // Horizontal pass: sh rows of dw pixels.
    let hc = contributions(sw, dw, r);
    let mut mid = vec![[0.0f32; 4]; dw * sh];
    mid.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        let line = &src.data[y * sw..(y + 1) * sw];
        for (x, out) in row.iter_mut().enumerate() {
            let c = &hc[x];
            let mut acc = [0.0f32; 4];
            for (k, &w) in c.weights.iter().enumerate() {
                let p = line[c.start + k];
                for ch in 0..4 {
                    acc[ch] += p[ch] * w;
                }
            }
            *out = acc;
        }
    });

    // Vertical pass.
    let vc = contributions(sh, dh, r);
    let mut out = ImageF32::new(width, height);
    out.data
        .par_chunks_mut(dw)
        .enumerate()
        .for_each(|(y, row)| {
            let c = &vc[y];
            for (x, o) in row.iter_mut().enumerate() {
                let mut acc = [0.0f32; 4];
                for (k, &w) in c.weights.iter().enumerate() {
                    let p = mid[(c.start + k) * dw + x];
                    for ch in 0..4 {
                        acc[ch] += p[ch] * w;
                    }
                }
                *o = acc;
            }
        });

    // Lobed filters overshoot; keep results in gamut.
    for p in &mut out.data {
        let a = p[3].clamp(0.0, 1.0);
        p[3] = a;
        for ch in &mut p[..3] {
            *ch = ch.clamp(0.0, a); // premultiplied: colour <= alpha
        }
    }
    out.unpremultiply();
    Ok(out)
}

/// Box-filter downsample by an integer factor; edge blocks average what exists.
pub fn box_downsample(img: &ImageF32, factor: u32) -> ImageF32 {
    if factor <= 1 {
        return img.clone();
    }
    let f = factor as usize;
    let (sw, sh) = (img.width as usize, img.height as usize);
    let (dw, dh) = (sw.div_ceil(f), sh.div_ceil(f));
    let mut src = img.clone();
    src.premultiply();
    let mut out = ImageF32::new(dw as u32, dh as u32);
    out.data
        .par_chunks_mut(dw)
        .enumerate()
        .for_each(|(by, row)| {
            for (bx, o) in row.iter_mut().enumerate() {
                let mut acc = [0.0f32; 4];
                let mut n = 0.0;
                for y in by * f..((by + 1) * f).min(sh) {
                    for x in bx * f..((bx + 1) * f).min(sw) {
                        let p = src.data[y * sw + x];
                        for ch in 0..4 {
                            acc[ch] += p[ch];
                        }
                        n += 1.0;
                    }
                }
                *o = [acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n];
            }
        });
    out.unpremultiply();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, v: f32) -> ImageF32 {
        let mut i = ImageF32::new(w, h);
        i.data.fill([v, v, v, 1.0]);
        i
    }

    #[test]
    fn all_resamplers_preserve_flat_colour() {
        for r in [
            Resampler::Nearest,
            Resampler::Bilinear,
            Resampler::Bicubic,
            Resampler::Lanczos,
        ] {
            for (w, h) in [(5, 7), (40, 30), (13, 13)] {
                let out = resize(&flat(20, 20, 0.4), w, h, r).unwrap();
                assert_eq!((out.width, out.height), (w, h));
                for p in &out.data {
                    assert!(
                        (p[0] - 0.4).abs() < 1e-4 && (p[3] - 1.0).abs() < 1e-5,
                        "{r:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn nearest_upscale_repeats() {
        let mut img = ImageF32::new(2, 1);
        img.data = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
        let out = resize(&img, 4, 1, Resampler::Nearest).unwrap();
        let v: Vec<f32> = out.data.iter().map(|p| p[0]).collect();
        assert_eq!(v, vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn bilinear_downscale_averages() {
        let mut img = ImageF32::new(2, 1);
        img.data = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
        let out = resize(&img, 1, 1, Resampler::Bilinear).unwrap();
        assert!((out.data[0][0] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn box_downsample_ragged_edges() {
        let out = box_downsample(&flat(5, 3, 0.25), 2);
        assert_eq!((out.width, out.height), (3, 2));
        assert!(out.data.iter().all(|p| (p[0] - 0.25).abs() < 1e-6));
    }

    #[test]
    fn target_resolution() {
        assert_eq!(
            ResizeTarget::Percent(50.0).resolve(100, 40).unwrap(),
            (50, 20)
        );
        assert_eq!(ResizeTarget::Width(50).resolve(100, 40).unwrap(), (50, 20));
        assert_eq!(ResizeTarget::Height(20).resolve(100, 40).unwrap(), (50, 20));
        assert_eq!(
            ResizeTarget::Fit {
                max_width: 50,
                max_height: 50
            }
            .resolve(100, 40)
            .unwrap(),
            (50, 20)
        );
        assert_eq!(
            ResizeTarget::Fit {
                max_width: 500,
                max_height: 500
            }
            .resolve(100, 40)
            .unwrap(),
            (100, 40)
        );
        assert!(ResizeTarget::Percent(0.0).resolve(10, 10).is_err());
    }
}
