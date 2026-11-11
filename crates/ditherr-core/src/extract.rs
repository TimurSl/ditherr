//! Automatic palette extraction: median cut, octree and k-means.
//!
//! All three are deterministic: no randomness, and ties are broken by colour
//! value. Input is a sample of sRGB 8-bit pixels; output is at most `n`
//! distinct colours (fewer if the image has fewer).

use std::collections::BTreeMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::color::{linear_to_oklab, linear_to_rgb8, oklab_to_linear, rgb8_to_linear};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoMethod {
    MedianCut,
    Octree,
    KMeans,
}

/// Unique colours with pixel counts, in sorted order.
fn histogram(pixels: &[[u8; 3]]) -> Vec<([u8; 3], u64)> {
    let mut map: BTreeMap<[u8; 3], u64> = BTreeMap::new();
    for &p in pixels {
        *map.entry(p).or_insert(0) += 1;
    }
    map.into_iter().collect()
}

fn sort_palette(mut p: Vec<[u8; 3]>) -> Vec<[u8; 3]> {
    p.sort_by_key(|c| {
        let y = 2126 * u32::from(c[0]) + 7152 * u32::from(c[1]) + 722 * u32::from(c[2]);
        (y, *c)
    });
    p.dedup();
    p
}

pub fn extract(method: AutoMethod, pixels: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    match method {
        AutoMethod::MedianCut => median_cut(pixels, n),
        AutoMethod::Octree => octree(pixels, n),
        AutoMethod::KMeans => kmeans(pixels, n),
    }
}

pub fn median_cut(pixels: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    let hist = histogram(pixels);
    if hist.len() <= n {
        return sort_palette(hist.into_iter().map(|(c, _)| c).collect());
    }
    // Each box is a range of `hist` after sorting; keep boxes as owned vectors.
    let mut boxes: Vec<Vec<([u8; 3], u64)>> = vec![hist];
    while boxes.len() < n {
        // Split the box with the widest channel range (ties: most pixels, then index).
        let pick = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| (channel_range(b).1, b.iter().map(|e| e.1).sum::<u64>(), i))
            .max_by_key(|&(range, count, i)| (range, count, std::cmp::Reverse(i)));
        let Some((_, _, idx)) = pick else { break };
        let mut b = boxes.swap_remove(idx);
        let (ch, _) = channel_range(&b);
        b.sort_by_key(|&(c, _)| (c[ch], c));
        let total: u64 = b.iter().map(|e| e.1).sum();
        let mut acc = 0;
        let mut cut = 1;
        for (i, e) in b.iter().enumerate() {
            acc += e.1;
            if acc * 2 >= total {
                cut = (i + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let hi = b.split_off(cut);
        boxes.push(b);
        boxes.push(hi);
    }
    sort_palette(
        boxes
            .iter()
            .map(|b| {
                let total: u64 = b.iter().map(|e| e.1).sum();
                let mut sum = [0u64; 3];
                for &(c, n) in b {
                    for ch in 0..3 {
                        sum[ch] += u64::from(c[ch]) * n;
                    }
                }
                [
                    ((sum[0] + total / 2) / total) as u8,
                    ((sum[1] + total / 2) / total) as u8,
                    ((sum[2] + total / 2) / total) as u8,
                ]
            })
            .collect(),
    )
}

/// Returns (channel with the widest range, that range).
fn channel_range(b: &[([u8; 3], u64)]) -> (usize, u8) {
    let mut best = (0, 0);
    for ch in 0..3 {
        let lo = b.iter().map(|e| e.0[ch]).min().unwrap_or(0);
        let hi = b.iter().map(|e| e.0[ch]).max().unwrap_or(0);
        if hi - lo > best.1 {
            best = (ch, hi - lo);
        }
    }
    best
}

#[derive(Default, Clone)]
struct OctNode {
    children: [u32; 8],
    count: u64,
    sum: [u64; 3],
    depth: u8,
    has_children: bool,
}

const NONE: u32 = u32::MAX;

pub fn octree(pixels: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    let hist = histogram(pixels);
    if hist.len() <= n {
        return sort_palette(hist.into_iter().map(|(c, _)| c).collect());
    }
    let mut nodes = vec![OctNode {
        children: [NONE; 8],
        ..Default::default()
    }];
    for &(c, count) in &hist {
        let mut cur = 0usize;
        for depth in 0..8u8 {
            let shift = 7 - depth;
            let idx = usize::from(
                (((c[0] >> shift) & 1) << 2) | (((c[1] >> shift) & 1) << 1) | ((c[2] >> shift) & 1),
            );
            if nodes[cur].children[idx] == NONE {
                nodes.push(OctNode {
                    children: [NONE; 8],
                    depth: depth + 1,
                    ..Default::default()
                });
                let id = (nodes.len() - 1) as u32;
                nodes[cur].children[idx] = id;
                nodes[cur].has_children = true;
            }
            cur = nodes[cur].children[idx] as usize;
        }
        nodes[cur].count += count;
        for ch in 0..3 {
            nodes[cur].sum[ch] += u64::from(c[ch]) * count;
        }
    }
    // Leaves are the unique colours at depth 8.
    let mut leaves = hist.len();
    'reduce: for depth in (1..8u8).rev() {
        // Subtree counts for nodes at this depth, computed from their (leaf) children.
        let mut candidates: Vec<(u64, usize)> = Vec::new();
        for (id, node) in nodes.iter().enumerate() {
            if node.depth == depth && node.has_children {
                let count: u64 = node
                    .children
                    .iter()
                    .filter(|&&c| c != NONE)
                    .map(|&c| nodes[c as usize].count)
                    .sum();
                candidates.push((count, id));
            }
        }
        candidates.sort();
        for (_, id) in candidates {
            if leaves <= n {
                break 'reduce;
            }
            let kids: Vec<usize> = nodes[id]
                .children
                .iter()
                .filter(|&&c| c != NONE)
                .map(|&c| c as usize)
                .collect();
            let mut count = 0;
            let mut sum = [0u64; 3];
            for &k in &kids {
                count += nodes[k].count;
                for ch in 0..3 {
                    sum[ch] += nodes[k].sum[ch];
                }
            }
            nodes[id].count = count;
            nodes[id].sum = sum;
            nodes[id].children = [NONE; 8];
            nodes[id].has_children = false;
            leaves = leaves + 1 - kids.len();
        }
    }
    if leaves > n {
        // Fewer colours requested than the eight top-level octants.
        return median_cut(pixels, n);
    }
    // Collect leaves by walking from the root.
    let mut out = Vec::new();
    let mut stack = vec![0usize];
    while let Some(id) = stack.pop() {
        let node = &nodes[id];
        if node.has_children {
            stack.extend(
                node.children
                    .iter()
                    .filter(|&&c| c != NONE)
                    .map(|&c| c as usize),
            );
        } else if let Some(half) = node.count.checked_div(2).filter(|_| node.count > 0) {
            out.push([
                ((node.sum[0] + half) / node.count) as u8,
                ((node.sum[1] + half) / node.count) as u8,
                ((node.sum[2] + half) / node.count) as u8,
            ]);
        }
    }
    sort_palette(out)
}

/// k-means in Oklab on the colour histogram, seeded from median cut so the
/// result needs no random numbers.
pub fn kmeans(pixels: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    let hist = histogram(pixels);
    if hist.len() <= n {
        return sort_palette(hist.into_iter().map(|(c, _)| c).collect());
    }
    let points: Vec<([f32; 3], f32)> = hist
        .iter()
        .map(|&(c, w)| (linear_to_oklab(rgb8_to_linear(c)), w as f32))
        .collect();
    let mut centres: Vec<[f32; 3]> = median_cut(pixels, n)
        .into_iter()
        .map(|c| linear_to_oklab(rgb8_to_linear(c)))
        .collect();

    let d2 = |a: [f32; 3], b: [f32; 3]| {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
    };
    let mut assignment = vec![usize::MAX; points.len()];
    for _ in 0..24 {
        let new: Vec<usize> = points
            .par_iter()
            .map(|&(p, _)| {
                let mut best = (f32::INFINITY, 0);
                for (i, &c) in centres.iter().enumerate() {
                    let d = d2(p, c);
                    if d < best.0 {
                        best = (d, i);
                    }
                }
                best.1
            })
            .collect();
        if new == assignment {
            break;
        }
        assignment = new;
        let mut sums = vec![([0.0f32; 3], 0.0f32); centres.len()];
        for (&(p, w), &a) in points.iter().zip(&assignment) {
            for ch in 0..3 {
                sums[a].0[ch] += p[ch] * w;
            }
            sums[a].1 += w;
        }
        for (c, (s, w)) in centres.iter_mut().zip(sums) {
            if w > 0.0 {
                *c = [s[0] / w, s[1] / w, s[2] / w];
            }
        }
    }
    sort_palette(
        centres
            .into_iter()
            .map(|c| linear_to_rgb8(oklab_to_linear(c)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four tight, equally weighted colour clusters. (Median cut splits at the median
    /// population, so an even number of equal clusters is the case it must get right.)
    fn four_clusters() -> Vec<[u8; 3]> {
        let mut v = Vec::new();
        for i in 0..40u8 {
            v.push([200 + i % 5, 10, 10]);
            v.push([10, 200 + i % 5, 10]);
            v.push([10, 10, 200 + i % 5]);
            v.push([200 + i % 5, 200, 10]);
        }
        v
    }

    fn near(p: &[[u8; 3]], target: [u8; 3]) -> bool {
        p.iter()
            .any(|c| (0..3).all(|ch| c[ch].abs_diff(target[ch]) < 8))
    }

    #[test]
    fn few_colours_returned_exactly() {
        let px = vec![[1, 2, 3], [4, 5, 6], [1, 2, 3]];
        for m in [
            AutoMethod::MedianCut,
            AutoMethod::Octree,
            AutoMethod::KMeans,
        ] {
            let p = extract(m, &px, 8);
            assert_eq!(p, vec![[1, 2, 3], [4, 5, 6]], "{m:?}");
        }
    }

    #[test]
    fn finds_the_clusters() {
        let px = four_clusters();
        for m in [
            AutoMethod::MedianCut,
            AutoMethod::Octree,
            AutoMethod::KMeans,
        ] {
            let p = extract(m, &px, 4);
            assert_eq!(p.len(), 4, "{m:?}: {p:?}");
            assert!(near(&p, [202, 10, 10]), "{m:?}: {p:?}");
            assert!(near(&p, [10, 202, 10]), "{m:?}: {p:?}");
            assert!(near(&p, [10, 10, 202]), "{m:?}: {p:?}");
            assert!(near(&p, [202, 200, 10]), "{m:?}: {p:?}");
        }
    }

    #[test]
    fn never_exceeds_requested_size_and_is_deterministic() {
        let px: Vec<[u8; 3]> = (0..4000u32)
            .map(|i| {
                [
                    (i * 7 % 256) as u8,
                    (i * 13 % 256) as u8,
                    (i * 29 % 256) as u8,
                ]
            })
            .collect();
        for m in [
            AutoMethod::MedianCut,
            AutoMethod::Octree,
            AutoMethod::KMeans,
        ] {
            for n in [2, 5, 16, 100] {
                let a = extract(m, &px, n);
                assert!(
                    !a.is_empty() && a.len() <= n,
                    "{m:?} n={n}: got {}",
                    a.len()
                );
                assert_eq!(a, extract(m, &px, n), "{m:?} not deterministic");
            }
        }
    }
}
