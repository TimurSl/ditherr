//! Generates `src/dither/blue_noise_64.bin`: a 64x64 blue-noise threshold map made
//! with Ulichney's void-and-cluster method (Gaussian sigma 1.5, toroidal).
//!
//! The map is baked into the repository rather than generated at runtime so that
//! every platform and every build uses identical thresholds. Regenerate with:
//!
//! ```sh
//! cargo run --release -p ditherr-core --example gen_blue_noise
//! ```
//!
//! Output: 4096 little-endian `u16` ranks, a permutation of `0..4096`.

#![allow(clippy::needless_range_loop)]

use rand_core::{RngCore, SeedableRng};
use rand_pcg::Pcg64;

const N: usize = 64;
const SIZE: usize = N * N;
const SIGMA: f64 = 1.5;

struct Field {
    kernel: Vec<f64>,
    energy: Vec<f64>,
}

impl Field {
    fn new() -> Self {
        let wrap = |d: usize| d.min(N - d) as f64;
        let mut kernel = vec![0.0; SIZE];
        for dy in 0..N {
            for dx in 0..N {
                let d2 = wrap(dx).powi(2) + wrap(dy).powi(2);
                kernel[dy * N + dx] = (-d2 / (2.0 * SIGMA * SIGMA)).exp();
            }
        }
        Self {
            kernel,
            energy: vec![0.0; SIZE],
        }
    }

    fn add(&mut self, p: usize, sign: f64) {
        let (px, py) = (p % N, p / N);
        for y in 0..N {
            let ky = (y + N - py) % N;
            for x in 0..N {
                let kx = (x + N - px) % N;
                self.energy[y * N + x] += sign * self.kernel[ky * N + kx];
            }
        }
    }

    fn rebuild(&mut self, pattern: &[bool]) {
        self.energy.fill(0.0);
        for (p, &on) in pattern.iter().enumerate() {
            if on {
                self.add(p, 1.0);
            }
        }
    }

    /// Pixel set in `pattern` with the highest energy.
    fn tightest_cluster(&self, pattern: &[bool]) -> usize {
        let mut best = (f64::NEG_INFINITY, 0);
        for p in 0..SIZE {
            if pattern[p] && self.energy[p] > best.0 {
                best = (self.energy[p], p);
            }
        }
        best.1
    }

    /// Pixel clear in `pattern` with the lowest energy.
    fn largest_void(&self, pattern: &[bool]) -> usize {
        let mut best = (f64::INFINITY, 0);
        for p in 0..SIZE {
            if !pattern[p] && self.energy[p] < best.0 {
                best = (self.energy[p], p);
            }
        }
        best.1
    }
}

fn main() {
    let mut rng = Pcg64::seed_from_u64(0xD17_4E22);
    let ones = SIZE / 10;

    // Initial random pattern.
    let mut pattern = vec![false; SIZE];
    let mut placed = 0;
    while placed < ones {
        let p = (rng.next_u32() as usize) % SIZE;
        if !pattern[p] {
            pattern[p] = true;
            placed += 1;
        }
    }

    // Relax: move the tightest cluster into the largest void until stable.
    let mut field = Field::new();
    field.rebuild(&pattern);
    loop {
        let c = field.tightest_cluster(&pattern);
        pattern[c] = false;
        field.add(c, -1.0);
        let v = field.largest_void(&pattern);
        pattern[v] = true;
        field.add(v, 1.0);
        if v == c {
            break;
        }
    }
    let initial = pattern.clone();

    let mut rank = vec![0u16; SIZE];

    // Phase 1: remove ones, tightest cluster first, ranking ones-1 down to 0.
    let mut work = initial.clone();
    field.rebuild(&work);
    for r in (0..ones).rev() {
        let c = field.tightest_cluster(&work);
        rank[c] = r as u16;
        work[c] = false;
        field.add(c, -1.0);
    }

    // Phases 2 and 3: add ones to the largest void, ranking ones..SIZE.
    let mut work = initial;
    field.rebuild(&work);
    for r in ones..SIZE {
        let v = field.largest_void(&work);
        rank[v] = r as u16;
        work[v] = true;
        field.add(v, 1.0);
    }

    let bytes: Vec<u8> = rank.iter().flat_map(|r| r.to_le_bytes()).collect();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/dither/blue_noise_64.bin");
    std::fs::write(path, bytes).expect("write blue noise map");
    println!("wrote {path}");
}
