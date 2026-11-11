//! Error diffusion: one generic scanner driven by a kernel table.

use serde::{Deserialize, Serialize};

use super::{noise, ErrorClamp, Params, ScanDirection, WorkImage, SALT_JITTER};
use crate::quantizer::Quantizer;

/// The built-in diffusion kernels (spec §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKernel {
    FloydSteinberg,
    JarvisJudiceNinke,
    Stucki,
    Burkes,
    /// Three-row Sierra.
    Sierra,
    SierraTwoRow,
    SierraLite,
    Atkinson,
    StevensonArce,
}

impl ErrorKernel {
    pub const ALL: [ErrorKernel; 9] = [
        ErrorKernel::FloydSteinberg,
        ErrorKernel::JarvisJudiceNinke,
        ErrorKernel::Stucki,
        ErrorKernel::Burkes,
        ErrorKernel::Sierra,
        ErrorKernel::SierraTwoRow,
        ErrorKernel::SierraLite,
        ErrorKernel::Atkinson,
        ErrorKernel::StevensonArce,
    ];

    pub fn kernel(self) -> Kernel {
        // (dx, dy, weight); dx is relative to the scan direction, dy is the next line.
        let (taps, divisor): (&[(i32, i32, i32)], i32) = match self {
            ErrorKernel::FloydSteinberg => (&[(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)], 16),
            ErrorKernel::JarvisJudiceNinke => (
                &[
                    (1, 0, 7),
                    (2, 0, 5),
                    (-2, 1, 3),
                    (-1, 1, 5),
                    (0, 1, 7),
                    (1, 1, 5),
                    (2, 1, 3),
                    (-2, 2, 1),
                    (-1, 2, 3),
                    (0, 2, 5),
                    (1, 2, 3),
                    (2, 2, 1),
                ],
                48,
            ),
            ErrorKernel::Stucki => (
                &[
                    (1, 0, 8),
                    (2, 0, 4),
                    (-2, 1, 2),
                    (-1, 1, 4),
                    (0, 1, 8),
                    (1, 1, 4),
                    (2, 1, 2),
                    (-2, 2, 1),
                    (-1, 2, 2),
                    (0, 2, 4),
                    (1, 2, 2),
                    (2, 2, 1),
                ],
                42,
            ),
            ErrorKernel::Burkes => (
                &[
                    (1, 0, 8),
                    (2, 0, 4),
                    (-2, 1, 2),
                    (-1, 1, 4),
                    (0, 1, 8),
                    (1, 1, 4),
                    (2, 1, 2),
                ],
                32,
            ),
            ErrorKernel::Sierra => (
                &[
                    (1, 0, 5),
                    (2, 0, 3),
                    (-2, 1, 2),
                    (-1, 1, 4),
                    (0, 1, 5),
                    (1, 1, 4),
                    (2, 1, 2),
                    (-1, 2, 2),
                    (0, 2, 3),
                    (1, 2, 2),
                ],
                32,
            ),
            ErrorKernel::SierraTwoRow => (
                &[
                    (1, 0, 4),
                    (2, 0, 3),
                    (-2, 1, 1),
                    (-1, 1, 2),
                    (0, 1, 3),
                    (1, 1, 2),
                    (2, 1, 1),
                ],
                16,
            ),
            ErrorKernel::SierraLite => (&[(1, 0, 2), (-1, 1, 1), (0, 1, 1)], 4),
            // Atkinson deliberately diffuses only 6/8 of the error.
            ErrorKernel::Atkinson => (
                &[
                    (1, 0, 1),
                    (2, 0, 1),
                    (-1, 1, 1),
                    (0, 1, 1),
                    (1, 1, 1),
                    (0, 2, 1),
                ],
                8,
            ),
            ErrorKernel::StevensonArce => (
                &[
                    (2, 0, 32),
                    (-3, 1, 12),
                    (-1, 1, 26),
                    (1, 1, 30),
                    (3, 1, 16),
                    (-2, 2, 12),
                    (0, 2, 26),
                    (2, 2, 12),
                    (-3, 3, 5),
                    (-1, 3, 12),
                    (1, 3, 12),
                    (3, 3, 5),
                ],
                200,
            ),
        };
        Kernel {
            taps: taps
                .iter()
                .map(|&(dx, dy, w)| Tap {
                    dx,
                    dy,
                    weight: w as f32 / divisor as f32,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tap {
    pub dx: i32,
    pub dy: i32,
    /// Share of the error sent to this neighbour (already divided by the divisor).
    pub weight: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Kernel {
    pub taps: Vec<Tap>,
}

impl Kernel {
    pub fn total_weight(&self) -> f32 {
        self.taps.iter().map(|t| t.weight).sum()
    }
}

/// Runs error diffusion over `work`, returning one code per pixel.
pub fn diffuse(work: &WorkImage, quant: &Quantizer, kernel: &Kernel, params: &Params) -> Vec<u32> {
    let (w, h) = (work.width, work.height);
    let vertical = matches!(
        params.scan,
        ScanDirection::TopToBottom | ScanDirection::BottomToTop
    );
    let reversed = matches!(
        params.scan,
        ScanDirection::RightToLeft | ScanDirection::BottomToTop
    );
    // A "line" is a row for horizontal scans and a column for vertical ones.
    let (lines, positions) = if vertical { (w, h) } else { (h, w) };

    let mut buf = work.data.clone();
    let mut codes = vec![0u32; w * h];
    let clamp = match params.error_clamp {
        ErrorClamp::Auto => quant.error_limit(),
        ErrorClamp::Off => f32::INFINITY,
        ErrorClamp::Limit(l) => l,
    };

    for line in 0..lines {
        let backwards = reversed ^ (params.serpentine && line % 2 == 1);
        for k in 0..positions {
            let pos = if backwards { positions - 1 - k } else { k };
            let (x, y) = if vertical { (line, pos) } else { (pos, line) };
            let i = y * w + x;
            let v = buf[i];

            let mut vq = v;
            if params.jitter > 0.0 {
                let n = (noise(params.seed, x as u32, y as u32, SALT_JITTER) - 0.5) * params.jitter;
                vq = [v[0] + n, v[1] + n, v[2] + n];
            }
            let (code, snapped) = quant.nearest(vq);
            codes[i] = code;

            // Pass on only error the palette can compensate, scaled and clamped.
            let raw =
                quant.project_error([v[0] - snapped[0], v[1] - snapped[1], v[2] - snapped[2]]);
            let err = [
                (raw[0] * params.strength).clamp(-clamp, clamp),
                (raw[1] * params.strength).clamp(-clamp, clamp),
                (raw[2] * params.strength).clamp(-clamp, clamp),
            ];
            for tap in &kernel.taps {
                let dpos = if backwards { -tap.dx } else { tap.dx };
                let nl = line as i64 + i64::from(tap.dy);
                let np = pos as i64 + i64::from(dpos);
                if nl < 0 || nl >= lines as i64 || np < 0 || np >= positions as i64 {
                    continue;
                }
                let (nx, ny) = if vertical {
                    (nl as usize, np as usize)
                } else {
                    (np as usize, nl as usize)
                };
                let t = &mut buf[ny * w + nx];
                t[0] += err[0] * tap.weight;
                t[1] += err[1] * tap.weight;
                t[2] += err[2] * tap.weight;
            }
        }
    }
    codes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_weights_sum_correctly() {
        for k in ErrorKernel::ALL {
            let total = k.kernel().total_weight();
            let expected = if k == ErrorKernel::Atkinson {
                0.75
            } else {
                1.0
            };
            assert!((total - expected).abs() < 1e-5, "{k:?}: {total}");
        }
    }

    #[test]
    fn kernels_only_reach_forward() {
        // Every tap must land on an unprocessed pixel: later on the same line or a later line.
        for k in ErrorKernel::ALL {
            for t in k.kernel().taps {
                assert!(t.dy > 0 || (t.dy == 0 && t.dx > 0), "{k:?} {t:?}");
            }
        }
    }
}
