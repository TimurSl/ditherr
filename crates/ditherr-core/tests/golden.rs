//! Golden hashes: every algorithm and built-in palette is rendered from a fixed
//! synthetic picture and its output compared byte-for-byte (via a 64-bit hash)
//! with `tests/golden.txt`. A mismatch means engine output changed; if that is
//! intended, regenerate with `UPDATE_GOLDEN=1 cargo test -p ditherr-core --test golden`.
//!
//! The same file is checked on Linux, Windows and macOS in CI, which is what
//! proves the engine is deterministic across platforms.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use common::*;
use ditherr_core::presets::PRESETS;
use ditherr_core::{
    render, Algorithm, AutoMethod, ErrorClamp, ErrorKernel, Look, Metric, PaletteSpec, Params,
    Resampler, ResizeTarget, Rotation, ScanDirection, Step,
};

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden.txt");

fn cases() -> Vec<(String, Look)> {
    let mut v: Vec<(String, Look)> = Vec::new();
    let mut add = |name: String, look: Look| v.push((name, look));
    let fs = || Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg);

    for k in ErrorKernel::ALL {
        add(
            format!("diffusion/{k:?}/bw"),
            Look {
                algorithm: Algorithm::ErrorDiffusion(k),
                ..Look::default()
            },
        );
    }
    for (name, scan, serpentine) in [
        ("ltr", ScanDirection::LeftToRight, false),
        ("rtl", ScanDirection::RightToLeft, false),
        ("ttb", ScanDirection::TopToBottom, false),
        ("btt", ScanDirection::BottomToTop, false),
        ("ttb-serp", ScanDirection::TopToBottom, true),
    ] {
        add(
            format!("scan/{name}"),
            Look {
                algorithm: fs(),
                palette: PaletteSpec::Preset("pico8".into()),
                params: Params {
                    scan,
                    serpentine,
                    ..Params::default()
                },
                ..Look::default()
            },
        );
    }
    for size in [2u8, 4, 8, 16] {
        add(
            format!("ordered/bayer{size}/gray4"),
            Look {
                algorithm: Algorithm::Bayer { size },
                palette: PaletteSpec::Gray { levels: 4 },
                ..Look::default()
            },
        );
    }
    add(
        "ordered/blue/bw".into(),
        Look {
            algorithm: Algorithm::BlueNoise,
            ..Look::default()
        },
    );
    add(
        "ordered/white/bw".into(),
        Look {
            algorithm: Algorithm::WhiteNoise,
            params: Params {
                seed: 7,
                ..Params::default()
            },
            ..Look::default()
        },
    );
    add(
        "basic/threshold".into(),
        Look {
            algorithm: Algorithm::Threshold { level: 0.5 },
            ..Look::default()
        },
    );
    add(
        "basic/posterize/pico8".into(),
        Look {
            algorithm: Algorithm::Posterize,
            palette: PaletteSpec::Preset("pico8".into()),
            ..Look::default()
        },
    );
    for p in PRESETS {
        add(
            format!("preset/{}/fs", p.id),
            Look {
                algorithm: fs(),
                palette: PaletteSpec::Preset(p.id.into()),
                ..Look::default()
            },
        );
        add(
            format!("preset/{}/bayer8", p.id),
            Look {
                algorithm: Algorithm::Bayer { size: 8 },
                palette: PaletteSpec::Preset(p.id.into()),
                ..Look::default()
            },
        );
    }
    for (name, method) in [
        ("median-cut", AutoMethod::MedianCut),
        ("octree", AutoMethod::Octree),
        ("kmeans", AutoMethod::KMeans),
    ] {
        add(
            format!("auto/{name}/8"),
            Look {
                algorithm: fs(),
                palette: PaletteSpec::Auto { method, colors: 8 },
                ..Look::default()
            },
        );
    }
    for (name, metric) in [
        ("rgb", Metric::Rgb),
        ("weighted", Metric::WeightedRgb),
        ("lab76", Metric::Lab76),
        ("lab2000", Metric::Lab2000),
        ("oklab", Metric::Oklab),
    ] {
        add(
            format!("metric/{name}/c64"),
            Look {
                algorithm: fs(),
                metric,
                palette: PaletteSpec::Preset("c64".into()),
                ..Look::default()
            },
        );
    }
    add(
        "levels/rgb332".into(),
        Look {
            algorithm: fs(),
            palette: PaletteSpec::rgb332(),
            ..Look::default()
        },
    );
    add(
        "levels/rgb565".into(),
        Look {
            algorithm: fs(),
            palette: PaletteSpec::rgb565(),
            ..Look::default()
        },
    );
    add(
        "params/gamma-space".into(),
        Look {
            algorithm: fs(),
            params: Params {
                linear_light: false,
                ..Params::default()
            },
            ..Look::default()
        },
    );
    add(
        "params/jitter-clamp".into(),
        Look {
            algorithm: fs(),
            palette: PaletteSpec::Preset("gameboy".into()),
            params: Params {
                jitter: 0.3,
                error_clamp: ErrorClamp::Limit(0.25),
                seed: 3,
                ..Params::default()
            },
            ..Look::default()
        },
    );
    add(
        "params/strength-bias".into(),
        Look {
            algorithm: Algorithm::Bayer { size: 8 },
            params: Params {
                strength: 0.6,
                bias: 0.1,
                ..Params::default()
            },
            ..Look::default()
        },
    );
    add(
        "pixel-scale/4".into(),
        Look {
            algorithm: Algorithm::Bayer { size: 4 },
            palette: PaletteSpec::Preset("pico8".into()),
            pixel_scale: 4,
            ..Look::default()
        },
    );
    add(
        "preprocess/stack".into(),
        Look {
            algorithm: fs(),
            palette: PaletteSpec::Preset("gameboy".into()),
            preprocess: vec![
                Step::Exposure(0.5),
                Step::Contrast(0.3),
                Step::Gamma(1.2),
                Step::Levels {
                    black: 0.05,
                    white: 0.95,
                    gamma: 1.0,
                },
                Step::Saturation(1.4),
                Step::Grayscale {
                    weights: [0.3, 0.59, 0.11],
                },
                Step::Crop {
                    x: 4,
                    y: 2,
                    width: 50,
                    height: 40,
                },
                Step::Rotate(Rotation::Cw90),
                Step::FlipHorizontal,
                Step::Resize {
                    target: ResizeTarget::Percent(75.0),
                    resampler: Resampler::Lanczos,
                },
            ],
            ..Look::default()
        },
    );
    for (name, r) in [
        ("nearest", Resampler::Nearest),
        ("bilinear", Resampler::Bilinear),
        ("bicubic", Resampler::Bicubic),
        ("lanczos", Resampler::Lanczos),
    ] {
        add(
            format!("resize/{name}"),
            Look {
                algorithm: Algorithm::Bayer { size: 4 },
                preprocess: vec![Step::Resize {
                    target: ResizeTarget::Width(37),
                    resampler: r,
                }],
                ..Look::default()
            },
        );
    }
    v
}

fn read_golden() -> BTreeMap<String, String> {
    std::fs::read_to_string(GOLDEN)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn golden_hashes_match() {
    let src = test_image(64, 48);
    let actual: BTreeMap<String, String> = cases()
        .into_iter()
        .map(|(name, look)| {
            let out = render(&src, &look).unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, format!("{:016x}", hash_rendered(&out)))
        })
        .collect();

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let mut text = String::new();
        for (k, v) in &actual {
            writeln!(text, "{k} {v}").unwrap();
        }
        std::fs::write(GOLDEN, text).unwrap();
        return;
    }

    let expected = read_golden();
    assert!(
        !expected.is_empty(),
        "tests/golden.txt is missing; generate it with UPDATE_GOLDEN=1"
    );
    let mut problems = Vec::new();
    for (k, v) in &actual {
        match expected.get(k) {
            Some(e) if e == v => {}
            Some(e) => problems.push(format!("{k}: expected {e}, got {v}")),
            None => problems.push(format!("{k}: no golden entry")),
        }
    }
    for k in expected.keys().filter(|k| !actual.contains_key(*k)) {
        problems.push(format!("{k}: golden entry has no test case"));
    }
    assert!(
        problems.is_empty(),
        "golden mismatches:\n{}",
        problems.join("\n")
    );
}

#[test]
fn rendering_is_repeatable_run_to_run() {
    let src = test_image(64, 48);
    for (name, look) in cases() {
        assert!(
            render(&src, &look).unwrap() == render(&src, &look).unwrap(),
            "{name}"
        );
    }
}
