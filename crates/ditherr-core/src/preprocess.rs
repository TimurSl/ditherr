//! The pre-process stack: a list of [`Step`]s applied in order to the linear-light
//! working image before pixel-scale downsampling and quantization.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::buffer::ImageF32;
use crate::color::{linear_to_srgb, luminance, srgb_to_linear};
use crate::error::{Error, Result};
use crate::resize::{resize, Resampler, ResizeTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rotation {
    Cw90,
    Cw180,
    Cw270,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Step {
    /// Scale by stops: linear light is multiplied by `2^stops`.
    Exposure(f32),
    /// `-1..` ; 0 is unchanged. Pivots on mid-gray in sRGB-encoded space.
    Contrast(f32),
    /// Applied to sRGB-encoded values as `v^(1/gamma)`; 1 is unchanged, >1 brightens.
    Gamma(f32),
    /// Black/white points and midtone gamma on sRGB-encoded values (0..=1).
    Levels {
        black: f32,
        white: f32,
        gamma: f32,
    },
    /// 1 is unchanged, 0 is grayscale, >1 boosts. Mixes against Rec. 709 luminance.
    Saturation(f32),
    /// Weighted grayscale of linear RGB. Rec. 709 is `[0.2126, 0.7152, 0.0722]`.
    Grayscale {
        weights: [f32; 3],
    },
    Resize {
        target: ResizeTarget,
        resampler: Resampler,
    },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Rotate(Rotation),
    FlipHorizontal,
    FlipVertical,
}

impl Step {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParam(m.into()));
        match *self {
            Step::Exposure(v) | Step::Contrast(v) | Step::Saturation(v) if !v.is_finite() => {
                bad("step value must be finite")
            }
            Step::Contrast(c) if c <= -1.0 => bad("contrast must be > -1"),
            Step::Saturation(s) if s < 0.0 => bad("saturation must be >= 0"),
            Step::Gamma(g) if !(g > 0.0 && g.is_finite()) => bad("gamma must be > 0"),
            Step::Levels {
                black,
                white,
                gamma,
            } if !(white > black && gamma > 0.0 && gamma.is_finite()) => {
                bad("levels need white > black and gamma > 0")
            }
            Step::Crop { width, height, .. } if width == 0 || height == 0 => {
                bad("crop size must be non-zero")
            }
            _ => Ok(()),
        }
    }
}

/// Applies `steps` in order.
pub fn apply(img: &ImageF32, steps: &[Step]) -> Result<ImageF32> {
    let mut cur = img.clone();
    for step in steps {
        step.validate()?;
        cur = apply_step(cur, step)?;
        if cur.is_empty() {
            return Err(Error::EmptyImage);
        }
    }
    Ok(cur)
}

fn map_rgb(img: &mut ImageF32, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
    img.data.par_iter_mut().for_each(|p| {
        let c = f([p[0], p[1], p[2]]);
        p[0] = c[0];
        p[1] = c[1];
        p[2] = c[2];
    });
}

/// Applies `f` to each channel's sRGB-encoded value.
fn map_encoded(img: &mut ImageF32, f: impl Fn(f32) -> f32 + Sync) {
    map_rgb(img, |c| {
        [
            srgb_to_linear(f(linear_to_srgb(c[0]))),
            srgb_to_linear(f(linear_to_srgb(c[1]))),
            srgb_to_linear(f(linear_to_srgb(c[2]))),
        ]
    });
}

fn apply_step(mut img: ImageF32, step: &Step) -> Result<ImageF32> {
    match *step {
        Step::Exposure(stops) => {
            let k = libm::exp2f(stops);
            map_rgb(&mut img, |c| [c[0] * k, c[1] * k, c[2] * k]);
        }
        Step::Contrast(c) => {
            let k = 1.0 + c;
            map_encoded(&mut img, |e| ((e - 0.5) * k + 0.5).clamp(0.0, 1.0));
        }
        Step::Gamma(g) => {
            let inv = 1.0 / g;
            map_encoded(&mut img, |e| libm::powf(e, inv));
        }
        Step::Levels {
            black,
            white,
            gamma,
        } => {
            let inv = 1.0 / gamma;
            map_encoded(&mut img, |e| {
                libm::powf(((e - black) / (white - black)).clamp(0.0, 1.0), inv)
            });
        }
        Step::Saturation(s) => {
            map_rgb(&mut img, |c| {
                let y = luminance(c);
                [
                    (y + (c[0] - y) * s).max(0.0),
                    (y + (c[1] - y) * s).max(0.0),
                    (y + (c[2] - y) * s).max(0.0),
                ]
            });
        }
        Step::Grayscale { weights } => {
            map_rgb(&mut img, |c| {
                let y = (weights[0] * c[0] + weights[1] * c[1] + weights[2] * c[2]).max(0.0);
                [y, y, y]
            });
        }
        Step::Resize { target, resampler } => {
            let (w, h) = target.resolve(img.width, img.height)?;
            img = resize(&img, w, h, resampler)?;
        }
        Step::Crop {
            x,
            y,
            width,
            height,
        } => {
            if x >= img.width || y >= img.height {
                return Err(Error::InvalidParam(
                    "crop origin is outside the image".into(),
                ));
            }
            let w = width.min(img.width - x);
            let h = height.min(img.height - y);
            let mut out = ImageF32::new(w, h);
            for row in 0..h as usize {
                let s = (y as usize + row) * img.width as usize + x as usize;
                out.data[row * w as usize..(row + 1) * w as usize]
                    .copy_from_slice(&img.data[s..s + w as usize]);
            }
            img = out;
        }
        Step::Rotate(rot) => img = rotate(&img, rot),
        Step::FlipHorizontal => {
            let w = img.width as usize;
            for row in img.data.chunks_mut(w) {
                row.reverse();
            }
        }
        Step::FlipVertical => {
            let w = img.width as usize;
            let h = img.height as usize;
            for y in 0..h / 2 {
                for x in 0..w {
                    img.data.swap(y * w + x, (h - 1 - y) * w + x);
                }
            }
        }
    }
    Ok(img)
}

fn rotate(img: &ImageF32, rot: Rotation) -> ImageF32 {
    let (w, h) = (img.width as usize, img.height as usize);
    match rot {
        Rotation::Cw180 => {
            let mut out = img.clone();
            out.data.reverse();
            out
        }
        Rotation::Cw90 | Rotation::Cw270 => {
            let mut out = ImageF32::new(img.height, img.width);
            for y in 0..h {
                for x in 0..w {
                    // Output is h wide, w tall.
                    let (ox, oy) = if rot == Rotation::Cw90 {
                        (h - 1 - y, x)
                    } else {
                        (y, w - 1 - x)
                    };
                    out.data[oy * h + ox] = img.data[y * w + x];
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img_2x1() -> ImageF32 {
        let mut i = ImageF32::new(2, 1);
        i.data = vec![[0.1, 0.2, 0.3, 1.0], [0.9, 0.8, 0.7, 1.0]];
        i
    }

    #[test]
    fn exposure_one_stop_doubles_linear() {
        let out = apply(&img_2x1(), &[Step::Exposure(1.0)]).unwrap();
        assert!((out.data[0][0] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn neutral_steps_are_identity() {
        let src = img_2x1();
        let steps = [
            Step::Exposure(0.0),
            Step::Contrast(0.0),
            Step::Gamma(1.0),
            Step::Levels {
                black: 0.0,
                white: 1.0,
                gamma: 1.0,
            },
            Step::Saturation(1.0),
        ];
        let out = apply(&src, &steps).unwrap();
        for (a, b) in out.data.iter().zip(&src.data) {
            for ch in 0..3 {
                assert!((a[ch] - b[ch]).abs() < 1e-4, "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn zero_saturation_gives_equal_channels() {
        let out = apply(&img_2x1(), &[Step::Saturation(0.0)]).unwrap();
        for p in &out.data {
            assert!((p[0] - p[1]).abs() < 1e-6 && (p[1] - p[2]).abs() < 1e-6);
        }
    }

    #[test]
    fn grayscale_uses_weights() {
        let out = apply(
            &img_2x1(),
            &[Step::Grayscale {
                weights: [1.0, 0.0, 0.0],
            }],
        )
        .unwrap();
        assert!((out.data[0][1] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn levels_clip_to_black_and_white() {
        let mut i = ImageF32::new(2, 1);
        i.data = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
        let out = apply(
            &i,
            &[Step::Levels {
                black: 0.25,
                white: 0.75,
                gamma: 1.0,
            }],
        )
        .unwrap();
        assert!(out.data[0][0] < 1e-6 && (out.data[1][0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn geometry() {
        let mut i = ImageF32::new(3, 2);
        for (n, p) in i.data.iter_mut().enumerate() {
            *p = [n as f32, 0.0, 0.0, 1.0];
        }
        let ids = |img: &ImageF32| img.data.iter().map(|p| p[0] as u32).collect::<Vec<_>>();
        // 0 1 2
        // 3 4 5
        assert_eq!(
            ids(&apply(&i, &[Step::FlipHorizontal]).unwrap()),
            [2, 1, 0, 5, 4, 3]
        );
        assert_eq!(
            ids(&apply(&i, &[Step::FlipVertical]).unwrap()),
            [3, 4, 5, 0, 1, 2]
        );
        assert_eq!(
            ids(&apply(&i, &[Step::Rotate(Rotation::Cw180)]).unwrap()),
            [5, 4, 3, 2, 1, 0]
        );
        let r90 = apply(&i, &[Step::Rotate(Rotation::Cw90)]).unwrap();
        assert_eq!((r90.width, r90.height), (2, 3));
        assert_eq!(ids(&r90), [3, 0, 4, 1, 5, 2]);
        let r270 = apply(&i, &[Step::Rotate(Rotation::Cw270)]).unwrap();
        assert_eq!(ids(&r270), [2, 5, 1, 4, 0, 3]);
        let crop = apply(
            &i,
            &[Step::Crop {
                x: 1,
                y: 0,
                width: 5,
                height: 1,
            }],
        )
        .unwrap();
        assert_eq!(ids(&crop), [1, 2]);
        assert!(apply(
            &i,
            &[Step::Crop {
                x: 3,
                y: 0,
                width: 1,
                height: 1
            }]
        )
        .is_err());
    }

    #[test]
    fn invalid_params_rejected() {
        let i = img_2x1();
        assert!(apply(&i, &[Step::Gamma(0.0)]).is_err());
        assert!(apply(&i, &[Step::Contrast(-1.0)]).is_err());
        assert!(apply(
            &i,
            &[Step::Levels {
                black: 0.5,
                white: 0.5,
                gamma: 1.0
            }]
        )
        .is_err());
    }
}
