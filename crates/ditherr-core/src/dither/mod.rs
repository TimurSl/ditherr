//! Dithering algorithms and their shared parameters.
//!
//! Error diffusion is one generic scanner plus a kernel table; ordered
//! dithering is one thresholder plus a matrix; noise dithering reuses the
//! thresholder with a per-pixel hash. Adding a custom kernel or matrix needs
//! no new engine code.

mod diffusion;
mod maps;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub use diffusion::{diffuse, ErrorKernel, Kernel, Tap};
pub use maps::{bayer, bayer_ranks, blue_noise, ThresholdMap, BLUE_NOISE_SIZE};

use crate::color::{linear_to_srgb, luminance};
use crate::error::{Error, Result};
use crate::quantizer::Quantizer;

pub(crate) const SALT_WHITE: u32 = 1;
pub(crate) const SALT_JITTER: u32 = 2;

/// A deterministic hash of `(seed, x, y, salt)` in `0..1`. Because it depends
/// only on position, parallel and sequential runs agree.
pub fn noise(seed: u64, x: u32, y: u32, salt: u32) -> f32 {
    fn mix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    let pos = (u64::from(x) << 32) | u64::from(y);
    let h = mix(seed ^ mix(pos ^ (u64::from(salt) << 58)));
    (h >> 40) as f32 / 16_777_216.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScanDirection {
    #[default]
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

/// How much error a pixel may pass on to its neighbours (spec §4, "error clamping").
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ErrorClamp {
    /// Derived from the palette: the widest gap between its colours. Prevents
    /// colour bleeding with tinted palettes and leaves ordinary palettes alone.
    #[default]
    Auto,
    /// Pass on all error.
    Off,
    /// Clamp each channel of the passed-on error to `±limit` (working-space units).
    Limit(f32),
}

/// Parameters shared by all algorithms (spec §4); each algorithm uses those that apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// Error diffusion: share of the error that is diffused (1.0 = 100%, may exceed 1).
    /// Ordered and noise: scales the pattern's amplitude (0 disables dithering).
    pub strength: f32,
    /// Error diffusion: alternate the direction of every other line.
    pub serpentine: bool,
    pub scan: ScanDirection,
    /// Added to every working-space channel before dithering; positive is lighter.
    pub bias: f32,
    /// Noise amplitude (`0..=1`) that perturbs decisions to break up worms and stripes.
    pub jitter: f32,
    /// Error diffusion: limit on the error passed to neighbours.
    pub error_clamp: ErrorClamp,
    /// Do dithering arithmetic in linear light (recommended). Off uses sRGB-encoded values.
    pub linear_light: bool,
    /// Seed for noise and jitter.
    pub seed: u64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            strength: 1.0,
            serpentine: true,
            scan: ScanDirection::LeftToRight,
            bias: 0.0,
            jitter: 0.0,
            error_clamp: ErrorClamp::Auto,
            linear_light: true,
            seed: 0,
        }
    }
}

impl Params {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParam(m.into()));
        if !(self.strength.is_finite() && self.strength >= 0.0) {
            return bad("strength must be >= 0");
        }
        if !self.bias.is_finite() {
            return bad("bias must be finite");
        }
        if !(0.0..=1.0).contains(&self.jitter) {
            return bad("jitter must be between 0 and 1");
        }
        if matches!(self.error_clamp, ErrorClamp::Limit(c) if !(c >= 0.0 && c.is_finite())) {
            return bad("error clamp must be >= 0");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Algorithm {
    /// Two-tone: pixels at or above `level` (perceptual, `0..=1`) become the
    /// palette's lightest colour, the rest its darkest.
    Threshold {
        level: f32,
    },
    /// Nearest palette colour, no dithering.
    Posterize,
    ErrorDiffusion(ErrorKernel),
    /// Bayer matrix of side 2, 4, 8 or 16.
    Bayer {
        size: u8,
    },
    /// 64x64 void-and-cluster blue noise.
    BlueNoise,
    /// Per-pixel random thresholds.
    WhiteNoise,
}

impl Algorithm {
    pub fn validate(&self) -> Result<()> {
        match self {
            Algorithm::Bayer { size } if !matches!(size, 2 | 4 | 8 | 16) => Err(
                Error::InvalidParam("bayer size must be 2, 4, 8 or 16".into()),
            ),
            Algorithm::Threshold { level } if !(0.0..=1.0).contains(level) => Err(
                Error::InvalidParam("threshold level must be between 0 and 1".into()),
            ),
            _ => Ok(()),
        }
    }
}

/// Opaque working-space RGB, ready to dither.
#[derive(Debug, Clone)]
pub struct WorkImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<[f32; 3]>,
}

/// Dithers `work` with `algo`, returning one palette code per pixel.
pub fn run(work: &WorkImage, quant: &Quantizer, algo: &Algorithm, params: &Params) -> Vec<u32> {
    let codes = match algo {
        Algorithm::ErrorDiffusion(k) => diffuse(work, quant, &k.kernel(), params),
        Algorithm::Posterize => per_pixel(work, |_, _, p| quant.nearest(p).0),
        Algorithm::Threshold { level } => {
            let (dark, light) = quant.endpoints();
            let space = quant.space();
            per_pixel(work, |_, _, p| {
                let perceptual = linear_to_srgb(luminance(space.to_linear(p)));
                if perceptual >= *level {
                    light
                } else {
                    dark
                }
            })
        }
        Algorithm::Bayer { size } => {
            let map = bayer(usize::from(*size));
            ordered(work, quant, params, |x, y| map.get(x, y))
        }
        Algorithm::BlueNoise => {
            let map = blue_noise();
            ordered(work, quant, params, |x, y| map.get(x, y))
        }
        Algorithm::WhiteNoise => ordered(work, quant, params, |x, y| {
            noise(params.seed, x as u32, y as u32, SALT_WHITE)
        }),
    };
    debug_assert_eq!(codes.len(), work.width * work.height);
    codes
}

fn per_pixel(work: &WorkImage, f: impl Fn(usize, usize, [f32; 3]) -> u32 + Sync) -> Vec<u32> {
    let w = work.width;
    let mut codes = vec![0u32; w * work.height];
    codes.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, c) in row.iter_mut().enumerate() {
            *c = f(x, y, work.data[y * w + x]);
        }
    });
    codes
}

fn ordered(
    work: &WorkImage,
    quant: &Quantizer,
    params: &Params,
    threshold: impl Fn(usize, usize) -> f32 + Sync,
) -> Vec<u32> {
    per_pixel(work, |x, y, p| {
        let mut t = threshold(x, y);
        if params.jitter > 0.0 {
            t += (noise(params.seed, x as u32, y as u32, SALT_JITTER) - 0.5) * params.jitter;
        }
        quant.ordered(p, t, params.strength)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_uniform_and_repeatable() {
        let n = 200;
        let mut sum = 0.0;
        for y in 0..n {
            for x in 0..n {
                let v = noise(7, x, y, 1);
                assert!((0.0..1.0).contains(&v));
                assert_eq!(v, noise(7, x, y, 1));
                sum += v;
            }
        }
        let mean = sum / (n * n) as f32;
        assert!((mean - 0.5).abs() < 0.01, "{mean}");
        assert_ne!(noise(7, 3, 4, 1), noise(8, 3, 4, 1));
        assert_ne!(noise(7, 3, 4, 1), noise(7, 3, 4, 2));
    }

    #[test]
    fn param_validation() {
        assert!(Params::default().validate().is_ok());
        assert!(Params {
            strength: -1.0,
            ..Params::default()
        }
        .validate()
        .is_err());
        assert!(Params {
            jitter: 2.0,
            ..Params::default()
        }
        .validate()
        .is_err());
        assert!(Algorithm::Bayer { size: 3 }.validate().is_err());
        assert!(Algorithm::Bayer { size: 16 }.validate().is_ok());
    }
}
