//! Threshold maps for ordered dithering.

use std::sync::OnceLock;

/// A tiling threshold map; values are in `0..1`, evenly distributed.
#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdMap {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

impl ThresholdMap {
    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.values[(y % self.height) * self.width + x % self.width]
    }

    /// Builds a map from integer ranks `0..width*height`.
    fn from_ranks(width: usize, height: usize, ranks: impl Iterator<Item = u32>) -> Self {
        let n = (width * height) as f32;
        Self {
            width,
            height,
            values: ranks.map(|r| (r as f32 + 0.5) / n).collect(),
        }
    }
}

/// Bayer index matrix of side `n` (a power of two, 2..=16 here), values `0..n*n`.
pub fn bayer_ranks(n: usize) -> Vec<u32> {
    assert!(n.is_power_of_two() && n >= 2);
    let mut m = vec![0u32, 2, 3, 1];
    let mut side = 2;
    while side < n {
        let next = side * 2;
        let mut out = vec![0u32; next * next];
        for y in 0..side {
            for x in 0..side {
                let v = 4 * m[y * side + x];
                out[y * next + x] = v;
                out[y * next + x + side] = v + 2;
                out[(y + side) * next + x] = v + 3;
                out[(y + side) * next + x + side] = v + 1;
            }
        }
        m = out;
        side = next;
    }
    m
}

pub fn bayer(n: usize) -> ThresholdMap {
    ThresholdMap::from_ranks(n, n, bayer_ranks(n).into_iter())
}

const BLUE_NOISE_BYTES: &[u8] = include_bytes!("blue_noise_64.bin");
pub const BLUE_NOISE_SIZE: usize = 64;

/// 64x64 void-and-cluster blue noise, baked by `examples/gen_blue_noise.rs`.
pub fn blue_noise() -> &'static ThresholdMap {
    static MAP: OnceLock<ThresholdMap> = OnceLock::new();
    MAP.get_or_init(|| {
        let ranks = BLUE_NOISE_BYTES
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| u32::from(u16::from_le_bytes(b)));
        ThresholdMap::from_ranks(BLUE_NOISE_SIZE, BLUE_NOISE_SIZE, ranks)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bayer_2_and_4_match_reference() {
        assert_eq!(bayer_ranks(2), vec![0, 2, 3, 1]);
        assert_eq!(
            bayer_ranks(4),
            vec![0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5]
        );
    }

    #[test]
    fn bayer_is_a_permutation_for_all_sizes() {
        for n in [2, 4, 8, 16] {
            let mut r = bayer_ranks(n);
            r.sort_unstable();
            assert!(r.iter().enumerate().all(|(i, &v)| v == i as u32), "n={n}");
        }
    }

    #[test]
    fn blue_noise_is_a_permutation() {
        assert_eq!(
            BLUE_NOISE_BYTES.len(),
            BLUE_NOISE_SIZE * BLUE_NOISE_SIZE * 2
        );
        let m = blue_noise();
        let mut ranks: Vec<u32> = m
            .values
            .iter()
            .map(|&v| (v * (BLUE_NOISE_SIZE * BLUE_NOISE_SIZE) as f32) as u32)
            .collect();
        ranks.sort_unstable();
        assert!(ranks.iter().enumerate().all(|(i, &v)| v == i as u32));
    }

    /// A 50% threshold of blue noise must be evenly spread: few solid 2x2 blocks
    /// (random noise has ~256 in a 64x64 tile; the baked map has 8) and almost no
    /// variation in how many pixels fall in each 8x8 window (random: ~16; baked: ~1.3).
    #[test]
    fn blue_noise_is_evenly_spread() {
        let m = blue_noise();
        let n = BLUE_NOISE_SIZE;
        let on = |x: usize, y: usize| m.get(x, y) < 0.5;
        let mut blocks = 0;
        for y in 0..n {
            for x in 0..n {
                if on(x, y) && on(x + 1, y) && on(x, y + 1) && on(x + 1, y + 1) {
                    blocks += 1;
                }
            }
        }
        assert!(blocks <= 32, "{blocks} solid 2x2 blocks");

        let k = 8;
        let counts: Vec<f32> = (0..n / k)
            .flat_map(|by| (0..n / k).map(move |bx| (bx, by)))
            .map(|(bx, by)| {
                (0..k * k)
                    .filter(|i| on(bx * k + i % k, by * k + i / k))
                    .count() as f32
            })
            .collect();
        let mean = counts.iter().sum::<f32>() / counts.len() as f32;
        let var = counts.iter().map(|c| (c - mean) * (c - mean)).sum::<f32>() / counts.len() as f32;
        assert!(var < 4.0, "window variance {var}");
    }
}
