//! THROWAWAY SPIKE (M1): is "ordered pattern sampled in a continuous space across
//! UV islands" workable before M5 builds on it? Delete once M5 has real tests.
//!
//! Setup: two faces of a cube meet along a vertical edge. Their UV islands are
//! stored in a 72x32 atlas with a gap between them, and the right island is
//! rotated by a quarter turn, as an exporter is free to do.
//!
//! * **Naive:** the Bayer matrix is indexed by atlas coordinates.
//! * **Seam-aware:** the Bayer matrix is indexed by *unfolded* coordinates: the
//!   right face is unfolded across the shared edge into the front face's plane,
//!   so the pattern runs unbroken over the edge.
//!
//! To judge the result we do what a viewer does: walk the 3D surface across the
//! edge, read the dithered atlas through the UV mapping, and compare with what
//! dithering the unfolded surface directly would give.
//!
//! Set `SEAM_SPIKE_OUT=<dir>` to write the atlases and strips as PNGs.
//!
//! ## Findings
//!
//! * For islands that share a texel lattice up to quarter turns and integer
//!   offsets (same texel density, axis-aligned unfolding) the scheme is exact:
//!   zero mismatches, no visible seam, and it needs nothing beyond a per-island
//!   integer isometry.
//! * Naive atlas-space patterns break at the edge: with a 50% gray the pattern
//!   inverts across it, giving a visible line.
//! * Not covered, and needed for M5: islands at arbitrary angles, mirrored
//!   islands, differing texel densities, and edges where unfolding is not
//!   planar (vertices with angle defect). Those need the pattern defined in
//!   surface units (world scale) and sampled per texel, with density
//!   normalisation, rather than a lattice mapping. Chains of islands also need
//!   a consistent unfolding order; a closed ring of islands cannot be unfolded
//!   without a cut, which is where a seam must remain.

use ditherr_core::dither::bayer;
use ditherr_core::{IndexedImage, Rendered};

const S: usize = 32; // texels per face edge
const GAP: usize = 8; // atlas gap between islands
const ATLAS_W: usize = S + GAP + S;

/// Atlas texel -> unfolded lattice coordinates, or `None` in the gap.
fn atlas_to_unfolded(ax: usize, ay: usize) -> Option<(usize, usize)> {
    if ax < S {
        Some((ax, ay)) // front island is stored unrotated
    } else if ax >= S + GAP {
        // Right island is stored rotated: atlas = (S+GAP + (S-1-b), a),
        // where `a` is distance from the shared edge and `b` the height.
        let (a, b) = (ay, S - 1 - (ax - (S + GAP)));
        Some((S + a, b)) // unfolded: continues the front face past the edge
    } else {
        None
    }
}

/// The inverse mapping: where the surface point at unfolded (u, v) lives in the atlas.
fn unfolded_to_atlas(u: usize, v: usize) -> (usize, usize) {
    if u < S {
        (u, v)
    } else {
        let a = u - S;
        (S + GAP + (S - 1 - v), a)
    }
}

/// Dithers a flat `level` into the atlas; `1` is white.
fn dither_atlas(level: f32, seam_aware: bool) -> Vec<Option<u8>> {
    let map = bayer(8);
    let mut atlas = vec![None; ATLAS_W * S];
    for ay in 0..S {
        for ax in 0..ATLAS_W {
            let Some((u, v)) = atlas_to_unfolded(ax, ay) else {
                continue;
            };
            let (tx, ty) = if seam_aware { (u, v) } else { (ax, ay) };
            atlas[ay * ATLAS_W + ax] = Some(u8::from(level > map.get(tx, ty)));
        }
    }
    atlas
}

/// What a viewer sees walking across the edge: a 2S x S strip of the surface.
fn surface_strip(atlas: &[Option<u8>]) -> Vec<u8> {
    let mut strip = Vec::with_capacity(2 * S * S);
    for v in 0..S {
        for u in 0..2 * S {
            let (ax, ay) = unfolded_to_atlas(u, v);
            strip.push(atlas[ay * ATLAS_W + ax].expect("surface maps into an island"));
        }
    }
    strip
}

/// Dithering the unfolded surface directly: the pattern with no seam at all.
fn ideal_strip(level: f32) -> Vec<u8> {
    let map = bayer(8);
    (0..S)
        .flat_map(|v| (0..2 * S).map(move |u| (u, v)))
        .map(|(u, v)| u8::from(level > map.get(u, v)))
        .collect()
}

/// Rows where the two texels either side of the edge are equal. In a 50% Bayer
/// checkerboard neighbours always differ, so equal pairs are a visible stripe.
fn equal_across_seam(strip: &[u8]) -> usize {
    (0..S)
        .filter(|&v| strip[v * 2 * S + S - 1] == strip[v * 2 * S + S])
        .count()
}

fn mismatches(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

fn write_png(dir: &std::path::Path, name: &str, w: usize, h: usize, px: &[Option<u8>], scale: u32) {
    let img = IndexedImage {
        width: w as u32,
        height: h as u32,
        indices: px.iter().map(|p| p.unwrap_or(0)).collect(),
        palette: vec![[20, 20, 20], [240, 240, 240]],
    };
    let r = Rendered::Indexed(img).upscale(scale);
    ditherr_io::save(dir.join(name), &r).unwrap();
}

#[test]
fn seam_aware_pattern_is_continuous_across_a_rotated_island_edge() {
    let out = std::env::var_os("SEAM_SPIKE_OUT").map(std::path::PathBuf::from);
    for level in [0.5f32, 0.25, 0.8] {
        let ideal = ideal_strip(level);
        let naive_atlas = dither_atlas(level, false);
        let aware_atlas = dither_atlas(level, true);
        let naive = surface_strip(&naive_atlas);
        let aware = surface_strip(&aware_atlas);

        let (m_naive, m_aware) = (mismatches(&naive, &ideal), mismatches(&aware, &ideal));
        println!(
            "level {level}: mismatches vs ideal naive {m_naive}/{} aware {m_aware}; \
             equal pairs across seam naive {} aware {} ideal {}",
            2 * S * S,
            equal_across_seam(&naive),
            equal_across_seam(&aware),
            equal_across_seam(&ideal),
        );

        assert_eq!(
            m_aware, 0,
            "seam-aware dithering must match the unfolded ideal"
        );
        assert!(
            m_naive > S * S / 8,
            "naive should visibly differ (level {level})"
        );
        // The seam-aware pattern behaves across the edge exactly as the ideal does.
        assert_eq!(equal_across_seam(&aware), equal_across_seam(&ideal));

        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            let tag = (level * 100.0) as u32;
            write_png(
                dir,
                &format!("atlas-naive-{tag}.png"),
                ATLAS_W,
                S,
                &naive_atlas,
                8,
            );
            write_png(
                dir,
                &format!("atlas-aware-{tag}.png"),
                ATLAS_W,
                S,
                &aware_atlas,
                8,
            );
            let some = |s: &[u8]| s.iter().map(|&v| Some(v)).collect::<Vec<_>>();
            write_png(
                dir,
                &format!("strip-naive-{tag}.png"),
                2 * S,
                S,
                &some(&naive),
                8,
            );
            write_png(
                dir,
                &format!("strip-aware-{tag}.png"),
                2 * S,
                S,
                &some(&aware),
                8,
            );
        }
    }
}

#[test]
fn naive_pattern_inverts_across_the_edge_at_half_gray() {
    // The headline artefact: at 50% the checkerboard flips phase at the seam, so every
    // row has two equal neighbours there.
    let naive = surface_strip(&dither_atlas(0.5, false));
    assert_eq!(equal_across_seam(&naive), S);
    let aware = surface_strip(&dither_atlas(0.5, true));
    assert_eq!(equal_across_seam(&aware), 0);
}

#[test]
fn mappings_are_inverses() {
    for v in 0..S {
        for u in 0..2 * S {
            let (ax, ay) = unfolded_to_atlas(u, v);
            assert_eq!(atlas_to_unfolded(ax, ay), Some((u, v)));
        }
    }
}
