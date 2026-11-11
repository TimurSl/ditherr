//! Colour spaces and distance metrics.
//!
//! All non-linear maths goes through `libm` so results are bit-identical on
//! every platform (see `clippy.toml`).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub fn srgb_to_linear(e: f32) -> f32 {
    if e <= 0.04045 {
        e / 12.92
    } else {
        libm::powf((e + 0.055) / 1.055, 2.4)
    }
}

/// Encodes linear light to sRGB. Input is clamped to `0..=1`.
pub fn linear_to_srgb(l: f32) -> f32 {
    let l = l.clamp(0.0, 1.0);
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * libm::powf(l, 1.0 / 2.4) - 0.055
    }
}

fn lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0.0; 256];
        for (i, v) in t.iter_mut().enumerate() {
            *v = srgb_to_linear(i as f32 / 255.0);
        }
        t
    })
}

pub fn srgb8_to_linear(v: u8) -> f32 {
    lut()[usize::from(v)]
}

pub fn linear_to_srgb8(l: f32) -> u8 {
    (linear_to_srgb(l) * 255.0 + 0.5) as u8
}

pub fn rgb8_to_linear(c: [u8; 3]) -> [f32; 3] {
    [
        srgb8_to_linear(c[0]),
        srgb8_to_linear(c[1]),
        srgb8_to_linear(c[2]),
    ]
}

pub fn linear_to_rgb8(c: [f32; 3]) -> [u8; 3] {
    [
        linear_to_srgb8(c[0]),
        linear_to_srgb8(c[1]),
        linear_to_srgb8(c[2]),
    ]
}

/// Rec. 709 relative luminance of linear RGB.
pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn linear_to_oklab(c: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = c;
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l, m, s) = (libm::cbrtf(l), libm::cbrtf(m), libm::cbrtf(s));
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn oklab_to_linear(c: [f32; 3]) -> [f32; 3] {
    let [l, a, b] = c;
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_93 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s,
        -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

/// CIELAB (D65) from linear sRGB.
pub fn linear_to_lab(c: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = c;
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
    let z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f32| {
        if t > 0.008_856 {
            libm::cbrtf(t)
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// CIEDE2000 colour difference, squared. Inputs are CIELAB triples.
pub fn delta_e_2000_sq(lab1: [f32; 3], lab2: [f32; 3]) -> f32 {
    const POW25_7: f32 = 6_103_515_625.0; // 25^7
    let [l1, a1, b1] = lab1;
    let [l2, a2, b2] = lab2;

    let c1 = libm::sqrtf(a1 * a1 + b1 * b1);
    let c2 = libm::sqrtf(a2 * a2 + b2 * b2);
    let cbar = (c1 + c2) / 2.0;
    let cbar7 = libm::powf(cbar, 7.0);
    let g = 0.5 * (1.0 - libm::sqrtf(cbar7 / (cbar7 + POW25_7)));
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = libm::sqrtf(a1p * a1p + b1 * b1);
    let c2p = libm::sqrtf(a2p * a2p + b2 * b2);

    let hue = |b: f32, ap: f32| {
        if b == 0.0 && ap == 0.0 {
            0.0
        } else {
            let h = libm::atan2f(b, ap).to_degrees();
            if h < 0.0 {
                h + 360.0
            } else {
                h
            }
        }
    };
    let h1p = hue(b1, a1p);
    let h2p = hue(b2, a2p);

    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let cprod = c1p * c2p;
    let dhp = if cprod == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d > 180.0 {
            d - 360.0
        } else if d < -180.0 {
            d + 360.0
        } else {
            d
        }
    };
    let dhp_big = 2.0 * libm::sqrtf(cprod) * libm::sinf((dhp / 2.0).to_radians());

    let lbarp = (l1 + l2) / 2.0;
    let cbarp = (c1p + c2p) / 2.0;
    let hbarp = if cprod == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };

    let cosd = |deg: f32| libm::cosf(deg.to_radians());
    let t =
        1.0 - 0.17 * cosd(hbarp - 30.0) + 0.24 * cosd(2.0 * hbarp) + 0.32 * cosd(3.0 * hbarp + 6.0)
            - 0.20 * cosd(4.0 * hbarp - 63.0);
    let dtheta = 30.0 * libm::expf(-((hbarp - 275.0) / 25.0) * ((hbarp - 275.0) / 25.0));
    let cbarp7 = libm::powf(cbarp, 7.0);
    let rc = 2.0 * libm::sqrtf(cbarp7 / (cbarp7 + POW25_7));
    let lm50 = (lbarp - 50.0) * (lbarp - 50.0);
    let sl = 1.0 + 0.015 * lm50 / libm::sqrtf(20.0 + lm50);
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -libm::sinf((2.0 * dtheta).to_radians()) * rc;

    let (x, y, z) = (dlp / sl, dcp / sc, dhp_big / sh);
    x * x + y * y + z * z + rt * y * z
}

/// How "nearest palette colour" is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Metric {
    /// Euclidean distance in sRGB-encoded RGB.
    Rgb,
    /// "Redmean" weighted RGB, a cheap perceptual approximation.
    WeightedRgb,
    /// CIELAB ΔE 1976.
    Lab76,
    /// CIELAB ΔE 2000.
    Lab2000,
    /// Euclidean distance in Oklab.
    #[default]
    Oklab,
}

impl Metric {
    /// Maps a linear RGB colour into this metric's space.
    pub fn coords(self, linear: [f32; 3]) -> [f32; 3] {
        let l = [
            linear[0].clamp(0.0, 1.0),
            linear[1].clamp(0.0, 1.0),
            linear[2].clamp(0.0, 1.0),
        ];
        match self {
            Metric::Rgb => [
                linear_to_srgb(l[0]),
                linear_to_srgb(l[1]),
                linear_to_srgb(l[2]),
            ],
            Metric::WeightedRgb => [
                linear_to_srgb(l[0]) * 255.0,
                linear_to_srgb(l[1]) * 255.0,
                linear_to_srgb(l[2]) * 255.0,
            ],
            Metric::Lab76 | Metric::Lab2000 => linear_to_lab(l),
            Metric::Oklab => linear_to_oklab(l),
        }
    }

    /// Squared distance between two points already mapped by [`coords`](Self::coords).
    /// Only the ordering of values is meaningful.
    pub fn dist2(self, a: [f32; 3], b: [f32; 3]) -> f32 {
        match self {
            Metric::Rgb | Metric::Lab76 | Metric::Oklab => dist2(a, b),
            Metric::WeightedRgb => {
                let rmean = (a[0] + b[0]) / 2.0;
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                (2.0 + rmean / 256.0) * d[0] * d[0]
                    + 4.0 * d[1] * d[1]
                    + (2.0 + (255.0 - rmean) / 256.0) * d[2] * d[2]
            }
            Metric::Lab2000 => delta_e_2000_sq(a, b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_roundtrip_all_bytes() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb8(srgb8_to_linear(v)), v);
        }
    }

    #[test]
    fn transfer_endpoints() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        assert!((linear_to_srgb(0.5) - 0.735_357).abs() < 1e-4);
    }

    #[test]
    fn oklab_white_and_roundtrip() {
        let w = linear_to_oklab([1.0, 1.0, 1.0]);
        assert!((w[0] - 1.0).abs() < 1e-3 && w[1].abs() < 1e-3 && w[2].abs() < 1e-3);
        let c = [0.2, 0.6, 0.9];
        let back = oklab_to_linear(linear_to_oklab(c));
        for i in 0..3 {
            assert!((back[i] - c[i]).abs() < 1e-4, "{back:?} vs {c:?}");
        }
    }

    #[test]
    fn lab_white_black() {
        let w = linear_to_lab([1.0, 1.0, 1.0]);
        assert!((w[0] - 100.0).abs() < 0.01 && w[1].abs() < 0.01 && w[2].abs() < 0.01);
        assert!(linear_to_lab([0.0; 3])[0].abs() < 1e-4);
    }

    /// Reference pairs from Sharma, Wu, Dalal (2005), "The CIEDE2000 colour-difference formula".
    #[test]
    fn delta_e_2000_reference_pairs() {
        let cases: [([f32; 3], [f32; 3], f32); 4] = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
            ([50.0, 2.5, 0.0], [50.0, 0.0, -2.5], 4.3065),
            (
                [60.2574, -34.0099, 36.2677],
                [60.4626, -34.1751, 39.4387],
                1.2644,
            ),
        ];
        for (a, b, expected) in cases {
            let d = delta_e_2000_sq(a, b).sqrt();
            assert!(
                (d - expected).abs() < 2e-3,
                "{a:?} {b:?}: {d} vs {expected}"
            );
            let r = delta_e_2000_sq(b, a).sqrt();
            assert!((d - r).abs() < 2e-3, "asymmetric");
        }
    }

    #[test]
    fn metrics_zero_for_identical_and_ordered() {
        for m in [
            Metric::Rgb,
            Metric::WeightedRgb,
            Metric::Lab76,
            Metric::Lab2000,
            Metric::Oklab,
        ] {
            let black = m.coords([0.0; 3]);
            let gray = m.coords([0.2; 3]);
            let white = m.coords([1.0; 3]);
            assert!(m.dist2(gray, gray) < 1e-9, "{m:?}");
            assert!(m.dist2(black, gray) < m.dist2(black, white), "{m:?}");
        }
    }
}
