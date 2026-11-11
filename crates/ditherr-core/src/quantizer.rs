//! Quantizers snap a working-space colour to the palette.
//!
//! Two kinds exist:
//!
//! * [`PaletteQuantizer`]: an explicit colour list, searched with a [`Metric`].
//! * [`GridQuantizer`]: a full grid of evenly spaced levels per channel (RGB
//!   3-3-2, RGB565, web-safe, EGA). Handled per channel, so it stays fast for
//!   palettes too large to search, and the metric does not apply.
//!
//! "Working space" is linear light when linear-light processing is on, and
//! sRGB-encoded otherwise; see [`Space`].

use crate::buffer::{IndexedImage, Rendered, Rgba8Image};
use crate::color::{linear_to_srgb, luminance, rgb8_to_linear, srgb_to_linear, Metric};

/// The colour space dithering arithmetic happens in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub linear: bool,
}

impl Space {
    pub fn from_linear(self, lin: [f32; 3]) -> [f32; 3] {
        if self.linear {
            lin
        } else {
            [
                linear_to_srgb(lin[0]),
                linear_to_srgb(lin[1]),
                linear_to_srgb(lin[2]),
            ]
        }
    }

    pub fn to_linear(self, w: [f32; 3]) -> [f32; 3] {
        if self.linear {
            w
        } else {
            [
                srgb_to_linear(w[0].clamp(0.0, 1.0)),
                srgb_to_linear(w[1].clamp(0.0, 1.0)),
                srgb_to_linear(w[2].clamp(0.0, 1.0)),
            ]
        }
    }

    /// Working-space value of an 8-bit sRGB colour.
    pub fn from_rgb8(self, c: [u8; 3]) -> [f32; 3] {
        if self.linear {
            rgb8_to_linear(c)
        } else {
            [
                f32::from(c[0]) / 255.0,
                f32::from(c[1]) / 255.0,
                f32::from(c[2]) / 255.0,
            ]
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaletteQuantizer {
    colors: Vec<[u8; 3]>,
    working: Vec<[f32; 3]>,
    coords: Vec<[f32; 3]>,
    metric: Metric,
    space: Space,
    /// Longest edge of the palette's minimum spanning tree in working space.
    max_gap: f32,
    /// Projects an error vector onto the directions the palette can compensate;
    /// `None` when it spans all three.
    error_projection: Option<[[f32; 3]; 3]>,
}

#[derive(Debug, Clone)]
pub struct GridQuantizer {
    /// Working-space level values per channel, ascending.
    levels: [Vec<f32>; 3],
    /// The 8-bit sRGB value of each level per channel.
    levels8: [Vec<u8>; 3],
    space: Space,
}

#[derive(Debug, Clone)]
pub enum Quantizer {
    Palette(PaletteQuantizer),
    Grid(GridQuantizer),
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Length of the longest edge in the Euclidean minimum spanning tree of `points`
/// (Prim's algorithm). This is the widest gap a dither has to bridge.
fn longest_mst_edge(points: &[[f32; 3]]) -> f32 {
    let n = points.len();
    let mut best = vec![f32::INFINITY; n];
    let mut done = vec![false; n];
    best[0] = 0.0;
    let mut longest = 0.0f32;
    for _ in 0..n {
        let next = (0..n)
            .filter(|&i| !done[i])
            .min_by(|&a, &b| best[a].total_cmp(&best[b]))
            .unwrap();
        done[next] = true;
        longest = longest.max(best[next]);
        for i in 0..n {
            if !done[i] {
                let d = sub(points[i], points[next]);
                best[i] = best[i].min(dot(d, d).sqrt());
            }
        }
    }
    longest
}

/// Eigen-decomposition of a symmetric 3x3 matrix by cyclic Jacobi rotations.
/// Returns the eigenvalues and the matching eigenvectors as columns of `v`.
fn symmetric_eigen(m: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..32 {
        let (p, q) = [(0, 1), (0, 2), (1, 2)]
            .into_iter()
            .max_by(|&(i, j), &(k, l)| a[i][j].abs().total_cmp(&a[k][l].abs()))
            .unwrap();
        if a[p][q].abs() < 1e-15 {
            break;
        }
        let theta = 0.5 * libm::atan2(2.0 * a[p][q], a[q][q] - a[p][p]);
        let (c, s) = (libm::cos(theta), libm::sin(theta));
        for k in 0..3 {
            let (akp, akq) = (a[k][p], a[k][q]);
            a[k][p] = c * akp - s * akq;
            a[k][q] = s * akp + c * akq;
        }
        for k in 0..3 {
            let (apk, aqk) = (a[p][k], a[q][k]);
            a[p][k] = c * apk - s * aqk;
            a[q][k] = s * apk + c * aqk;
        }
        for row in &mut v {
            let (vp, vq) = (row[p], row[q]);
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

/// Palettes whose colours lie close to a line or plane (black and white, gray
/// ramps, a Game Boy-style tinted ramp) cannot reproduce error in the other
/// directions, however long the dither runs. This builds the projection onto the
/// axes along which the palette has real spread (eigenvalue of the colour
/// covariance at least 1% of the largest), or `None` if it spans all three.
fn compensable_projection(colors: &[[f32; 3]]) -> Option<[[f32; 3]; 3]> {
    const KEEP_RATIO: f64 = 0.01;
    let n = colors.len() as f64;
    let mut mean = [0.0f64; 3];
    for c in colors {
        for i in 0..3 {
            mean[i] += f64::from(c[i]) / n;
        }
    }
    let mut cov = [[0.0f64; 3]; 3];
    for c in colors {
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += (f64::from(c[i]) - mean[i]) * (f64::from(c[j]) - mean[j]) / n;
            }
        }
    }
    let (values, vectors) = symmetric_eigen(cov);
    let largest = values.iter().copied().fold(0.0, f64::max);
    let mut proj = [[0.0f64; 3]; 3];
    let mut kept = 0;
    for (k, &lambda) in values.iter().enumerate() {
        if largest > 1e-12 && lambda >= KEEP_RATIO * largest {
            kept += 1;
            for i in 0..3 {
                for j in 0..3 {
                    proj[i][j] += vectors[i][k] * vectors[j][k];
                }
            }
        }
    }
    if kept == 3 {
        return None;
    }
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = proj[i][j] as f32;
        }
    }
    Some(out)
}

impl PaletteQuantizer {
    pub fn new(colors: Vec<[u8; 3]>, metric: Metric, space: Space) -> Self {
        assert!(!colors.is_empty(), "palette must not be empty");
        let working: Vec<[f32; 3]> = colors.iter().map(|&c| space.from_rgb8(c)).collect();
        let coords = colors
            .iter()
            .map(|&c| metric.coords(rgb8_to_linear(c)))
            .collect();
        let max_gap = longest_mst_edge(&working);
        let error_projection = compensable_projection(&working);
        Self {
            colors,
            working,
            coords,
            metric,
            space,
            max_gap,
            error_projection,
        }
    }

    fn nearest_index(&self, p: [f32; 3]) -> usize {
        let q = self.metric.coords(self.space.to_linear(p));
        let mut best = (f32::INFINITY, 0);
        for (i, &c) in self.coords.iter().enumerate() {
            let d = self.metric.dist2(q, c);
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }

    /// Ordered-dither choice. Finds the nearest colour `c1`, then the nearest
    /// colour `c2` lying in the direction of `p` from `c1`, and picks between
    /// the two by comparing where `p` falls on the segment with the threshold.
    /// This is exact for ramps (gray levels, duotone) and a sound approximation
    /// for arbitrary palettes. `t` is in `0..1`; `strength` scales the pattern.
    fn ordered(&self, p: [f32; 3], t: f32, strength: f32) -> usize {
        let i1 = self.nearest_index(p);
        let c1 = self.working[i1];
        let pv = sub(p, c1);
        let mut best: Option<(f32, usize)> = None;
        for (j, &cj) in self.working.iter().enumerate() {
            if j == i1 || dot(sub(cj, c1), pv) <= 0.0 {
                continue;
            }
            let d = sub(cj, p);
            let d = dot(d, d);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, j));
            }
        }
        let Some((_, j)) = best else { return i1 };
        let dv = sub(self.working[j], c1);
        let frac = (dot(pv, dv) / dot(dv, dv)).clamp(0.0, 1.0);
        if frac - 0.5 > (t - 0.5) * strength {
            j
        } else {
            i1
        }
    }
}

impl GridQuantizer {
    pub fn new(r: u16, g: u16, b: u16, space: Space) -> Self {
        let make = |n: u16| -> (Vec<f32>, Vec<u8>) {
            let n = n.max(2);
            let l8: Vec<u8> = (0..n)
                .map(|k| {
                    ((u32::from(k) * 255 * 2 + u32::from(n - 1)) / (2 * u32::from(n - 1))) as u8
                })
                .collect();
            let lv = l8.iter().map(|&v| space.from_rgb8([v, v, v])[0]).collect();
            (lv, l8)
        };
        let (lr, r8) = make(r);
        let (lg, g8) = make(g);
        let (lb, b8) = make(b);
        Self {
            levels: [lr, lg, lb],
            levels8: [r8, g8, b8],
            space,
        }
    }

    pub fn dims(&self) -> [usize; 3] {
        [
            self.levels[0].len(),
            self.levels[1].len(),
            self.levels[2].len(),
        ]
    }

    fn code(&self, idx: [usize; 3]) -> u32 {
        let d = self.dims();
        ((idx[0] * d[1] + idx[1]) * d[2] + idx[2]) as u32
    }

    fn nearest_level(levels: &[f32], v: f32) -> usize {
        let hi = levels.partition_point(|&l| l < v);
        if hi == 0 {
            0
        } else if hi == levels.len() || v - levels[hi - 1] <= levels[hi] - v {
            hi - 1
        } else {
            hi
        }
    }

    fn nearest(&self, p: [f32; 3]) -> [usize; 3] {
        [
            Self::nearest_level(&self.levels[0], p[0]),
            Self::nearest_level(&self.levels[1], p[1]),
            Self::nearest_level(&self.levels[2], p[2]),
        ]
    }

    fn ordered(&self, p: [f32; 3], t: f32, strength: f32) -> [usize; 3] {
        let mut out = [0; 3];
        for ch in 0..3 {
            let lv = &self.levels[ch];
            let v = p[ch].clamp(lv[0], lv[lv.len() - 1]);
            let hi = lv.partition_point(|&l| l < v).clamp(1, lv.len() - 1);
            let (a, b) = (lv[hi - 1], lv[hi]);
            let frac = (v - a) / (b - a);
            out[ch] = if frac - 0.5 > (t - 0.5) * strength {
                hi
            } else {
                hi - 1
            };
        }
        out
    }

    fn working_of(&self, idx: [usize; 3]) -> [f32; 3] {
        [
            self.levels[0][idx[0]],
            self.levels[1][idx[1]],
            self.levels[2][idx[2]],
        ]
    }

    fn decode(&self, code: u32) -> [usize; 3] {
        let d = self.dims();
        let code = code as usize;
        [code / (d[1] * d[2]), (code / d[2]) % d[1], code % d[2]]
    }
}

impl Quantizer {
    pub fn palette(colors: Vec<[u8; 3]>, metric: Metric, space: Space) -> Self {
        Quantizer::Palette(PaletteQuantizer::new(colors, metric, space))
    }

    pub fn grid(r: u16, g: u16, b: u16, space: Space) -> Self {
        Quantizer::Grid(GridQuantizer::new(r, g, b, space))
    }

    /// Number of colours the quantizer can produce.
    pub fn len(&self) -> usize {
        match self {
            Quantizer::Palette(p) => p.colors.len(),
            Quantizer::Grid(g) => g.dims().iter().product(),
        }
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Keeps only the part of an error vector the palette can compensate: for a
    /// palette along a line (black and white, gray ramps) that is the component
    /// along it, so chroma error is not diffused. See [`compensable_projection`].
    pub fn project_error(&self, e: [f32; 3]) -> [f32; 3] {
        match self {
            Quantizer::Palette(PaletteQuantizer {
                error_projection: Some(m),
                ..
            }) => [
                m[0][0] * e[0] + m[0][1] * e[1] + m[0][2] * e[2],
                m[1][0] * e[0] + m[1][1] * e[1] + m[1][2] * e[2],
                m[2][0] * e[0] + m[2][1] * e[1] + m[2][2] * e[2],
            ],
            _ => e,
        }
    }

    /// The error (per channel, working space) that diffusion may pass on when the
    /// clamp is automatic.
    ///
    /// A palette that cannot reproduce the source's hue, such as a green Game Boy
    /// palette fed a gray image, leaves an error no neighbour can ever cancel; left
    /// unchecked it grows without bound and swamps every decision ("colour
    /// bleeding"). The widest gap between palette colours bounds any error a pixel
    /// inside the palette's range can legitimately produce, so black and white,
    /// gray ramps and grids are unaffected. Grids cover the whole colour cube and
    /// need no limit.
    pub fn error_limit(&self) -> f32 {
        match self {
            Quantizer::Palette(q) => q.max_gap,
            Quantizer::Grid(_) => f32::INFINITY,
        }
    }

    /// Snaps `p` to the nearest palette colour. Returns its code and its
    /// working-space value.
    pub fn nearest(&self, p: [f32; 3]) -> (u32, [f32; 3]) {
        match self {
            Quantizer::Palette(q) => {
                let i = q.nearest_index(p);
                (i as u32, q.working[i])
            }
            Quantizer::Grid(q) => {
                let idx = q.nearest(p);
                (q.code(idx), q.working_of(idx))
            }
        }
    }

    /// Ordered-dither decision for threshold `t` (`0..1`). See
    /// [`PaletteQuantizer`] for the semantics of `strength`.
    pub fn ordered(&self, p: [f32; 3], t: f32, strength: f32) -> u32 {
        match self {
            Quantizer::Palette(q) => q.ordered(p, t, strength) as u32,
            Quantizer::Grid(q) => q.code(q.ordered(p, t, strength)),
        }
    }

    /// The darkest and lightest colours (by luminance), as codes.
    pub fn endpoints(&self) -> (u32, u32) {
        match self {
            Quantizer::Palette(q) => {
                let y = |i: usize| luminance(rgb8_to_linear(q.colors[i]));
                let mut dark = 0;
                let mut light = 0;
                for i in 1..q.colors.len() {
                    if y(i) < y(dark) {
                        dark = i;
                    }
                    if y(i) > y(light) {
                        light = i;
                    }
                }
                (dark as u32, light as u32)
            }
            Quantizer::Grid(q) => {
                let d = q.dims();
                (q.code([0, 0, 0]), q.code([d[0] - 1, d[1] - 1, d[2] - 1]))
            }
        }
    }

    pub fn color_of(&self, code: u32) -> [u8; 3] {
        match self {
            Quantizer::Palette(q) => q.colors[code as usize],
            Quantizer::Grid(q) => {
                let i = q.decode(code);
                [q.levels8[0][i[0]], q.levels8[1][i[1]], q.levels8[2][i[2]]]
            }
        }
    }

    /// All colours in code order.
    pub fn colors(&self) -> Vec<[u8; 3]> {
        (0..self.len() as u32).map(|c| self.color_of(c)).collect()
    }

    pub fn space(&self) -> Space {
        match self {
            Quantizer::Palette(q) => q.space,
            Quantizer::Grid(q) => q.space,
        }
    }

    /// Packages per-pixel codes: indexed when the palette fits in 8 bits.
    pub fn finish(&self, codes: &[u32], width: u32, height: u32) -> Rendered {
        let colors = self.colors();
        if colors.len() <= 256 {
            Rendered::Indexed(IndexedImage {
                width,
                height,
                indices: codes.iter().map(|&c| c as u8).collect(),
                palette: colors,
            })
        } else {
            let mut data = Vec::with_capacity(codes.len() * 4);
            for &c in codes {
                let rgb = self.color_of(c);
                data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            Rendered::Truecolor(Rgba8Image {
                width,
                height,
                data,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIN: Space = Space { linear: true };
    const ENC: Space = Space { linear: false };

    fn bw(space: Space) -> Quantizer {
        Quantizer::palette(vec![[0, 0, 0], [255, 255, 255]], Metric::Oklab, space)
    }

    #[test]
    fn nearest_picks_closest_palette_colour() {
        let q = Quantizer::palette(vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]], Metric::Rgb, ENC);
        assert_eq!(q.nearest([0.9, 0.1, 0.0]).0, 1);
        assert_eq!(q.nearest([0.1, 0.8, 0.1]).0, 2);
        assert_eq!(q.nearest([0.05, 0.05, 0.05]).0, 0);
    }

    #[test]
    fn ordered_ramp_probability_equals_fraction() {
        // Linear-light black/white: input 0.3 should come out white 30% of the time.
        let q = bw(LIN);
        let n = 1000;
        let white = (0..n)
            .filter(|&i| q.ordered([0.3; 3], (i as f32 + 0.5) / n as f32, 1.0) == 1)
            .count();
        assert_eq!(white, 300);
    }

    #[test]
    fn ordered_ramp_multi_level_gray() {
        // 3 grays in linear light: 0, ~0.214, 1. A value between the upper two.
        let q = Quantizer::palette(
            vec![[0, 0, 0], [128, 128, 128], [255, 255, 255]],
            Metric::Oklab,
            LIN,
        );
        let mid = srgb_to_linear(128.0 / 255.0);
        let p = mid + 0.25 * (1.0 - mid);
        let n = 1000;
        let up = (0..n)
            .filter(|&i| q.ordered([p; 3], (i as f32 + 0.5) / n as f32, 1.0) == 2)
            .count();
        assert!((up as i32 - 250).abs() <= 1, "{up}");
    }

    #[test]
    fn ordered_strength_zero_is_nearest_of_pair() {
        let q = bw(LIN);
        assert_eq!(q.ordered([0.4; 3], 0.01, 0.0), 0);
        assert_eq!(q.ordered([0.6; 3], 0.99, 0.0), 1);
    }

    #[test]
    fn grid_332_and_565() {
        let q = Quantizer::grid(8, 8, 4, ENC);
        assert_eq!(q.len(), 256);
        let q565 = Quantizer::grid(32, 64, 32, ENC);
        assert_eq!(q565.len(), 65536);
        let (code, snapped) = q565.nearest([1.0, 1.0, 1.0]);
        assert_eq!(q565.color_of(code), [255, 255, 255]);
        assert_eq!(snapped, [1.0, 1.0, 1.0]);
        // Every code round-trips through colour decode.
        let (dark, light) = q565.endpoints();
        assert_eq!(q565.color_of(dark), [0, 0, 0]);
        assert_eq!(q565.color_of(light), [255, 255, 255]);
    }

    #[test]
    fn grid_levels_match_expected_values() {
        let q = Quantizer::grid(6, 6, 6, ENC); // web-safe
        let colors = q.colors();
        assert_eq!(colors.len(), 216);
        assert!(colors.contains(&[51, 102, 204]));
        let q4 = Quantizer::grid(4, 4, 4, ENC); // EGA 64
        assert!(q4.colors().contains(&[85, 170, 255]));
    }

    #[test]
    fn grid_ordered_probability_per_channel() {
        let q = Quantizer::grid(2, 2, 2, LIN);
        let n = 1000;
        let mut hits = [0; 3];
        for i in 0..n {
            let code = q.ordered([0.1, 0.5, 0.9], (i as f32 + 0.5) / n as f32, 1.0);
            let [r, g, b] = {
                let c = q.color_of(code);
                [c[0] > 0, c[1] > 0, c[2] > 0]
            };
            hits[0] += usize::from(r);
            hits[1] += usize::from(g);
            hits[2] += usize::from(b);
        }
        assert_eq!(hits, [100, 500, 900]);
    }

    #[test]
    fn finish_is_indexed_up_to_256_colours() {
        let q = Quantizer::grid(8, 8, 4, ENC);
        assert!(matches!(q.finish(&[0], 1, 1), Rendered::Indexed(_)));
        let q = Quantizer::grid(32, 64, 32, ENC);
        assert!(matches!(q.finish(&[0], 1, 1), Rendered::Truecolor(_)));
    }

    #[test]
    fn error_limit_is_the_widest_gap() {
        // Gray ramp in encoded space: gaps of 0.2 and 0.8, so the limit is the wider one.
        let q = Quantizer::palette(
            vec![[0, 0, 0], [51, 51, 51], [255, 255, 255]],
            Metric::Rgb,
            ENC,
        );
        let expected = 0.8 * 3f32.sqrt();
        assert!(
            (q.error_limit() - expected).abs() < 1e-5,
            "{}",
            q.error_limit()
        );
        assert!(Quantizer::grid(8, 8, 4, ENC).error_limit().is_infinite());
        // A single colour has no gaps at all.
        assert_eq!(
            Quantizer::palette(vec![[9, 9, 9]], Metric::Rgb, ENC).error_limit(),
            0.0
        );
    }

    #[test]
    fn jacobi_finds_known_eigenvalues() {
        // Eigenvalues 1, 3, 4 (from a rotated diagonal matrix).
        let m = [[2.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 0.0, 4.0]];
        let (mut vals, _) = symmetric_eigen(m);
        vals.sort_by(f64::total_cmp);
        for (v, e) in vals.iter().zip([1.0, 3.0, 4.0]) {
            assert!((v - e).abs() < 1e-9, "{vals:?}");
        }
    }

    #[test]
    fn error_projection_follows_the_palette_shape() {
        // Black and white: only the gray component of an error survives.
        let bw = bw(ENC);
        let e = bw.project_error([0.3, 0.0, 0.0]);
        for c in e {
            assert!((c - 0.1).abs() < 1e-6, "{e:?}");
        }
        // A pure chroma error (zero mean) vanishes entirely.
        let e = bw.project_error([0.2, -0.1, -0.1]);
        assert!(e.iter().all(|c| c.abs() < 1e-6), "{e:?}");

        // Full-colour palettes pass errors through unchanged.
        let rgb = Quantizer::palette(
            vec![
                [0, 0, 0],
                [255, 0, 0],
                [0, 255, 0],
                [0, 0, 255],
                [255, 255, 255],
            ],
            Metric::Rgb,
            ENC,
        );
        assert_eq!(rgb.project_error([0.3, -0.2, 0.1]), [0.3, -0.2, 0.1]);
        assert_eq!(
            Quantizer::grid(8, 8, 4, ENC).project_error([0.3, -0.2, 0.1]),
            [0.3, -0.2, 0.1]
        );

        // A Game Boy-style tinted ramp is, in linear light, almost a single line with
        // hardly any blue: blue error is discarded.
        let gb = Quantizer::palette(
            vec![[15, 56, 15], [48, 98, 48], [139, 172, 15], [155, 188, 15]],
            Metric::Rgb,
            LIN,
        );
        let e = gb.project_error([0.0, 0.0, 0.5]);
        assert!(e[2].abs() < 0.05, "{e:?}");
    }

    #[test]
    fn endpoints_of_palette() {
        let q = Quantizer::palette(
            vec![[100, 100, 100], [0, 0, 40], [250, 250, 250]],
            Metric::Rgb,
            ENC,
        );
        assert_eq!(q.endpoints(), (1, 2));
    }
}
