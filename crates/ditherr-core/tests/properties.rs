mod common;

use common::*;
use ditherr_core::color::{luminance, rgb8_to_linear};
use ditherr_core::{
    render, Algorithm, ErrorClamp, ErrorKernel, Look, PaletteSpec, Params, Rendered, ScanDirection,
};

fn bw_look(algorithm: Algorithm) -> Look {
    Look {
        algorithm,
        palette: PaletteSpec::black_white(),
        ..Look::default()
    }
}

/// Mean linear luminance of a rendered image.
fn mean_luminance(r: &Rendered) -> f32 {
    let rgba = r.to_rgba8();
    let sum: f32 = rgba
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| luminance(rgb8_to_linear([p[0], p[1], p[2]])))
        .sum();
    sum / (rgba.data.len() / 4) as f32
}

const MID_GRAY_LINEAR: f32 = 0.215_860_5; // sRGB 128

#[test]
fn flat_mid_gray_keeps_its_mean_luminance_in_linear_light() {
    let src = flat_gray(128, 128, 128);
    // (algorithm, tolerance)
    let mut cases: Vec<(Algorithm, f32)> =
        vec![(Algorithm::BlueNoise, 0.005), (Algorithm::WhiteNoise, 0.02)];
    for k in ErrorKernel::ALL {
        // Atkinson deliberately discards a quarter of the error, so it drifts.
        if k != ErrorKernel::Atkinson {
            cases.push((Algorithm::ErrorDiffusion(k), 0.01));
        }
    }
    for size in [2u8, 4, 8, 16] {
        let n = f32::from(size) * f32::from(size);
        cases.push((Algorithm::Bayer { size }, 0.5 / n + 0.005));
    }
    for (algo, tol) in cases {
        let out = render(&src, &bw_look(algo.clone())).unwrap();
        let mean = mean_luminance(&out);
        assert!(
            (mean - MID_GRAY_LINEAR).abs() <= tol,
            "{algo:?}: mean {mean}, wanted {MID_GRAY_LINEAR} ± {tol}"
        );
    }
}

#[test]
fn without_linear_light_midgray_comes_out_too_bright() {
    let src = flat_gray(128, 128, 128);
    let mut look = bw_look(Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg));
    look.params.linear_light = false;
    let mean = mean_luminance(&render(&src, &look).unwrap());
    assert!(mean > 0.4, "mean {mean}");
}

#[test]
fn output_only_contains_palette_colours() {
    let src = test_image(48, 40);
    let presets = [
        "gameboy",
        "cga",
        "c64",
        "pico8",
        "zx-spectrum",
        "ega",
        "web-safe",
    ];
    for id in presets {
        for algo in [
            Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg),
            Algorithm::Bayer { size: 4 },
            Algorithm::BlueNoise,
            Algorithm::Posterize,
        ] {
            let look = Look {
                algorithm: algo.clone(),
                palette: PaletteSpec::Preset(id.into()),
                ..Look::default()
            };
            let out = render(&src, &look).unwrap();
            let Rendered::Indexed(img) = &out else {
                panic!("{id} should be indexed")
            };
            let allowed: std::collections::HashSet<_> = img.palette.iter().copied().collect();
            for p in out.to_rgba8().data.as_chunks::<4>().0 {
                assert!(allowed.contains(&[p[0], p[1], p[2]]), "{id} {algo:?}");
            }
            assert!(img
                .indices
                .iter()
                .all(|&i| usize::from(i) < img.palette.len()));
        }
    }
}

#[test]
fn rgb565_output_sits_on_the_level_grid() {
    let look = Look {
        palette: PaletteSpec::rgb565(),
        ..Look::default()
    };
    let out = render(&test_image(40, 30), &look).unwrap();
    let Rendered::Truecolor(img) = out else {
        panic!("565 is truecolour")
    };
    let on_grid =
        |v: u8, n: u32| (0..n).any(|k| (k * 255 * 2 + (n - 1)) / (2 * (n - 1)) == u32::from(v));
    for p in img.data.as_chunks::<4>().0 {
        assert!(
            on_grid(p[0], 32) && on_grid(p[1], 64) && on_grid(p[2], 32),
            "{p:?}"
        );
    }
}

#[test]
fn fixed_seed_is_repeatable_and_seeds_differ() {
    let src = test_image(48, 40);
    for algo in [
        Algorithm::WhiteNoise,
        Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg),
    ] {
        let mk = |seed| Look {
            algorithm: algo.clone(),
            params: Params {
                seed,
                jitter: 0.5,
                ..Params::default()
            },
            ..Look::default()
        };
        let a = render(&src, &mk(1)).unwrap();
        assert!(
            a == render(&src, &mk(1)).unwrap(),
            "{algo:?} not repeatable"
        );
        assert!(a != render(&src, &mk(2)).unwrap(), "{algo:?} ignores seed");
    }
}

#[test]
fn strength_zero_and_zero_clamp_equal_posterize() {
    let src = test_image(48, 40);
    let posterize = render(&src, &bw_look(Algorithm::Posterize)).unwrap();
    for params in [
        Params {
            strength: 0.0,
            ..Params::default()
        },
        Params {
            error_clamp: ErrorClamp::Limit(0.0),
            ..Params::default()
        },
    ] {
        let look = Look {
            algorithm: Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg),
            params,
            ..Look::default()
        };
        assert!(render(&src, &look).unwrap() == posterize);
    }
}

/// Ordered strength 0 removes the pattern: the result no longer depends on the matrix.
#[test]
fn ordered_strength_zero_ignores_the_matrix() {
    let src = test_image(48, 40);
    let at = |algorithm| {
        let look = Look {
            algorithm,
            params: Params {
                strength: 0.0,
                ..Params::default()
            },
            ..Look::default()
        };
        render(&src, &look).unwrap()
    };
    let bayer = at(Algorithm::Bayer { size: 8 });
    assert!(bayer == at(Algorithm::Bayer { size: 2 }));
    assert!(bayer == at(Algorithm::BlueNoise));
}

#[test]
fn bias_shifts_balance() {
    let src = flat_gray(64, 64, 128);
    let mean = |bias| {
        let mut look = bw_look(Algorithm::Bayer { size: 8 });
        look.params.bias = bias;
        mean_luminance(&render(&src, &look).unwrap())
    };
    assert!(mean(0.2) > mean(0.0) + 0.1);
    assert!(mean(-0.2) < mean(0.0) - 0.1);
}

#[test]
fn threshold_is_two_tone_and_respects_level() {
    let src = test_image(48, 40);
    let count_white = |level| {
        let out = render(&src, &bw_look(Algorithm::Threshold { level })).unwrap();
        let Rendered::Indexed(img) = out else {
            panic!()
        };
        img.indices.iter().filter(|&&i| i == 1).count()
    };
    assert!(count_white(0.2) > count_white(0.5));
    assert!(count_white(0.5) > count_white(0.8));
}

/// Scanning right-to-left is the same as mirroring, scanning left-to-right and mirroring back.
#[test]
fn right_to_left_scan_is_the_mirror_of_left_to_right() {
    let src = test_image(41, 33);
    let look = |scan| Look {
        palette: PaletteSpec::Preset("pico8".into()),
        algorithm: Algorithm::ErrorDiffusion(ErrorKernel::JarvisJudiceNinke),
        params: Params {
            scan,
            serpentine: false,
            ..Params::default()
        },
        ..Look::default()
    };
    let rtl = rgba(&render(&src, &look(ScanDirection::RightToLeft)).unwrap());
    let mirrored = flip_h(&rgba(
        &render(&flip_h(&src), &look(ScanDirection::LeftToRight)).unwrap(),
    ));
    assert!(rtl == mirrored, "RTL is not the mirror of LTR");
}

/// Scanning top-to-bottom is the same as transposing, scanning left-to-right and transposing back.
#[test]
fn vertical_scan_is_the_transpose_of_horizontal() {
    let src = test_image(41, 33);
    let look = |scan| Look {
        palette: PaletteSpec::Preset("pico8".into()),
        algorithm: Algorithm::ErrorDiffusion(ErrorKernel::Stucki),
        params: Params {
            scan,
            serpentine: false,
            ..Params::default()
        },
        ..Look::default()
    };
    let vertical = rgba(&render(&src, &look(ScanDirection::TopToBottom)).unwrap());
    let transposed = transpose(&rgba(
        &render(&transpose(&src), &look(ScanDirection::LeftToRight)).unwrap(),
    ));
    assert!(
        vertical == transposed,
        "vertical scan is not the transpose of horizontal"
    );
}

#[test]
fn serpentine_changes_the_pattern() {
    let src = test_image(48, 40);
    let look = |serpentine| Look {
        params: Params {
            serpentine,
            ..Params::default()
        },
        ..Look::default()
    };
    assert!(render(&src, &look(true)).unwrap() != render(&src, &look(false)).unwrap());
}

#[test]
fn pixel_scale_blocks_are_uniform_after_upscale() {
    let src = test_image(48, 40);
    let look = Look {
        pixel_scale: 4,
        algorithm: Algorithm::Bayer { size: 4 },
        ..Look::default()
    };
    let small = render(&src, &look).unwrap();
    assert_eq!((small.width(), small.height()), (12, 10));
    let up = rgba(&small.upscale(4));
    for by in 0..10usize {
        for bx in 0..12usize {
            let at = |x: usize, y: usize| {
                let i = ((by * 4 + y) * 48 + bx * 4 + x) * 4;
                up.data[i..i + 3].to_vec()
            };
            for (x, y) in [(1, 0), (0, 3), (3, 3)] {
                assert_eq!(at(0, 0), at(x, y));
            }
        }
    }
}

#[test]
fn every_algorithm_handles_tiny_images() {
    let algos = [
        Algorithm::Threshold { level: 0.5 },
        Algorithm::Posterize,
        Algorithm::BlueNoise,
        Algorithm::WhiteNoise,
        Algorithm::Bayer { size: 16 },
    ]
    .into_iter()
    .chain(ErrorKernel::ALL.map(Algorithm::ErrorDiffusion));
    for algo in algos {
        for (w, h) in [(1, 1), (1, 5), (5, 1), (2, 2)] {
            let look = bw_look(algo.clone());
            let src = ditherr_core::Rgba8Image::new(w, h, vec![100; (w * h * 4) as usize]).unwrap();
            let out = render(&src, &look).unwrap();
            assert_eq!((out.width(), out.height()), (w, h), "{algo:?}");
        }
    }
}
